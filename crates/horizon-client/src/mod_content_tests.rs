use super::*;
use crate::{assets::Assets, blocks::BlockFacts, items::Items};

#[test]
fn renames_ids_keys_and_states() {
    let mut r = Rename::new("minecraft:iron_block", "ruby:ruby_block");
    r.states = 10..12;
    r.first = 100;
    assert_eq!(
        r.string("minecraft:iron_block").as_deref(),
        Some("ruby:ruby_block")
    );
    assert_eq!(
        r.string("Block{minecraft:iron_block}").as_deref(),
        Some("Block{ruby:ruby_block}")
    );
    assert_eq!(
        r.string("block.minecraft.iron_block").as_deref(),
        Some("block.ruby.ruby_block")
    );
    assert_eq!(r.string("minecraft:iron_blocks"), None);
    assert_eq!(r.string("block.minecraft.iron_bars"), None);
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(101), None)
    );
}

#[test]
fn json_keeps_key_order_and_numbers() {
    let text = r#"{"z":1,"a":[0.6,5.0,-1,{"m":null,"b":true}],"k":"v"}"#;
    let doc: J = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&doc).unwrap(), text);
}

#[test]
fn item_components_patch_the_template_entry() {
    let mut entry: J =
        serde_json::from_str(r#"{"minecraft:max_damage": 250, "minecraft:repairable": {}}"#)
            .unwrap();
    let known: BTreeSet<String> = [
        "minecraft:max_damage",
        "minecraft:repairable",
        "minecraft:food",
    ]
    .map(str::to_owned)
    .into();
    let patch = |text: &str| -> Vec<(String, J)> {
        let J::Obj(o) = serde_json::from_str(text).unwrap() else {
            unreachable!()
        };
        o
    };
    patch_components(
        &mut entry,
        &patch(r#"{"minecraft:max_damage": 900, "!minecraft:repairable": {}, "minecraft:food": {"nutrition": 4}}"#),
        &known,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&entry).unwrap(),
        r#"{"minecraft:max_damage":900,"minecraft:food":{"nutrition":4}}"#
    );
    for typo in [r#"{"minecraft:fod": {}}"#, r#"{"!minecraft:fod": {}}"#] {
        let e = patch_components(&mut entry, &patch(typo), &known).unwrap_err();
        assert_eq!(e, "minecraft:fod is not a known component type");
    }
}

#[test]
fn creative_tabs_keep_vanilla_items_missing_from_search() {
    let tab = horizon_mods::content::CreativeTab {
        id: "test:restricted".into(),
        title: "Restricted".into(),
        icon: "minecraft:barrier".into(),
        items: [
            "minecraft:barrier",
            "minecraft:command_block",
            "minecraft:written_book",
            "minecraft:knowledge_book",
        ]
        .map(str::to_owned)
        .into(),
    };
    let plan = Plan {
        blocks: Vec::new(),
        items: Vec::new(),
        vanilla_items: tab.items.iter().cloned().collect(),
        effects: Vec::new(),
        entity_types: Vec::new(),
        menu_types: Vec::new(),
        states: 0,
        defaults: BTreeMap::new(),
        sounds: BTreeMap::new(),
        tabs: vec![&tab],
        names: BTreeMap::new(),
    };
    let mut doc: J = serde_json::from_str(
        r#"{"tabs":[{"type":"SEARCH","items":["{count:1,id:\"minecraft:stone\"}"]}]}"#,
    )
    .unwrap();
    plan.creative_tabs(&mut doc).unwrap();
    let doc: serde_json::Value = serde_json::to_value(doc).unwrap();
    assert_eq!(
        doc["tabs"][1]["items"],
        serde_json::json!([
            "{count:1,id:\"minecraft:barrier\"}",
            "{count:1,id:\"minecraft:command_block\"}",
            "{count:1,id:\"minecraft:written_book\"}",
            "{count:1,id:\"minecraft:knowledge_book\"}",
        ])
    );
}

/// A block item's patch goes over its template block's item, and a type no
/// vanilla item has fails the build.
#[test]
fn block_items_patch_their_template_item() {
    let scratch = std::env::temp_dir().join(format!("horizon-block-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"]], "defaults": {"minecraft:iron_block": 0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item": "minecraft:iron_block"}]}"#,
    )
    .unwrap();
    let items = r##"{"minecraft:iron_block": {"minecraft:max_stack_size": 64,
        "minecraft:item_name": {"translate": "block.minecraft.iron_block"}},
        "minecraft:netherite_block": {"minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}}"##;
    let block = |dir: &str, components: &str| {
        folder_mod(
            &scratch.join(dir),
            &[
                ("horizon-mod.toml", &manifest("ruby", "")),
                (
                    "data/ruby/horizon/block/ruby_block.json",
                    &format!(
                        r#"{{"template": "minecraft:iron_block", "components": {components}}}"#
                    ),
                ),
            ],
        )
    };
    let mods = [block(
        "ruby",
        r##"{"minecraft:max_stack_size": 16, "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}"##,
    )];
    let mut doc: J = serde_json::from_str(items).unwrap();
    Plan::new(&base, &mods)
        .unwrap()
        .json("items.json", &mut doc)
        .unwrap();
    let doc: serde_json::Value = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        doc["ruby:ruby_block"],
        serde_json::json!({"minecraft:max_stack_size": 16,
            "minecraft:item_name": {"translate": "block.ruby.ruby_block"},
            "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}})
    );
    assert_eq!(doc["minecraft:iron_block"]["minecraft:max_stack_size"], 64);
    let mods = [block("typo", r#"{"minecraft:fod": {}}"#)];
    let mut doc: J = serde_json::from_str(items).unwrap();
    let e = Plan::new(&base, &mods)
        .unwrap()
        .json("items.json", &mut doc)
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "ruby:ruby_block: minecraft:fod is not a known component type"
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn tags_merge_without_repeats() {
    let vanilla = r#"{"values": ["minecraft:stone"]}"#;
    let more = [
        J::Str("ruby:ruby_block".into()),
        J::Str("minecraft:stone".into()),
    ];
    let merged: serde_json::Value =
        serde_json::from_str(&merge_tag(Some(vanilla), &more, "t").unwrap()).unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "ruby:ruby_block"])
    );
}

/// Registry IDs and state IDs must not depend on package discovery order.
#[test]
fn allocated_ids_follow_mod_and_entry_order_after_vanilla() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-order-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"], ["minecraft:iron_block"], ["minecraft:stone"]],
            "defaults": {"minecraft:iron_block": 0, "minecraft:stone": 2}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item":"minecraft:iron_block"}, {"item":"minecraft:iron_block"},
            {"item":"minecraft:stone"}]}"#,
    )
    .unwrap();
    fs::write(
        base.join("items.json"),
        r#"{"minecraft:stone": {}, "minecraft:iron_block": {}}"#,
    )
    .unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":4},
            "minecraft:iron_block":{"protocol_id":9}}},
            "minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7},
            "minecraft:iron_block":{"protocol_id":11}}},
            "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0},
            "minecraft:luck":{"protocol_id":25}}},
            "minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1}}}}"#,
    )
    .unwrap();
    fs::create_dir_all(base.join("assets/minecraft/blockstates")).unwrap();
    fs::write(
        base.join("assets/minecraft/blockstates/iron_block.json"),
        "{}",
    )
    .unwrap();
    fs::write(base.join("assets/minecraft/blockstates/stone.json"), "{}").unwrap();
    let ruby = folder_mod(
        &scratch.join("ruby"),
        &[
            ("horizon-mod.toml", &manifest("ruby", "")),
            (
                "data/ruby/horizon/block/zblock.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/ruby/horizon/item/aitem.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/ruby/horizon/mob_effect/glow.json",
                r#"{"category":"beneficial","color":16711680}"#,
            ),
        ],
    );
    let alpha = folder_mod(
        &scratch.join("alpha"),
        &[
            ("horizon-mod.toml", &manifest("alpha", "")),
            (
                "data/alpha/horizon/block/zblock.json",
                r#"{"template":"minecraft:iron_block"}"#,
            ),
            (
                "data/alpha/horizon/item/aitem.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/alpha/horizon/mob_effect/zap.json",
                r#"{"category":"harmful","color":255}"#,
            ),
            (
                "data/alpha/horizon/mob_effect/dull.json",
                r#"{"category":"neutral","color":0}"#,
            ),
        ],
    );
    let mut mods = vec![ruby, alpha];
    for index in 0..2 {
        let plan = Plan::new(&base, &mods).unwrap();
        let blocks: Vec<_> = plan
            .blocks
            .iter()
            .map(|block| {
                (
                    block.decl.id.as_str(),
                    block.rename.first,
                    block.rename.states.len(),
                )
            })
            .collect();
        assert_eq!(blocks, [("alpha:zblock", 3, 2), ("ruby:zblock", 5, 1)]);
        let items: Vec<_> = plan
            .items
            .iter()
            .map(|item| item.rename.to.as_str())
            .collect();
        assert_eq!(
            items,
            ["alpha:aitem", "alpha:zblock", "ruby:aitem", "ruby:zblock"]
        );

        // Exercise the same registry rewrite the game and server read.
        let mut registry: J = serde_json::from_str(
            r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":4},
                "minecraft:iron_block":{"protocol_id":9}}},
                "minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7},
                "minecraft:iron_block":{"protocol_id":11}}},
                "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0},
                "minecraft:luck":{"protocol_id":25}}},
                "minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1}}}}"#,
        )
        .unwrap();
        plan.json("registries.json", &mut registry).unwrap();
        let registry: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&registry).unwrap()).unwrap();
        let id = |kind: &str, name: &str| {
            registry[kind]["entries"][name]["protocol_id"]
                .as_u64()
                .unwrap()
        };
        assert_eq!(id("minecraft:item", "minecraft:stone"), 4);
        assert_eq!(id("minecraft:item", "minecraft:iron_block"), 9);
        assert_eq!(id("minecraft:item", "alpha:aitem"), 10);
        assert_eq!(id("minecraft:item", "alpha:zblock"), 11);
        assert_eq!(id("minecraft:item", "ruby:aitem"), 12);
        assert_eq!(id("minecraft:item", "ruby:zblock"), 13);
        assert_eq!(id("minecraft:block", "minecraft:stone"), 7);
        assert_eq!(id("minecraft:block", "minecraft:iron_block"), 11);
        assert_eq!(id("minecraft:block", "alpha:zblock"), 12);
        assert_eq!(id("minecraft:block", "ruby:zblock"), 13);
        // Each mod block names a block entity type, after vanilla's.
        assert_eq!(id("minecraft:block_entity_type", "minecraft:chest"), 1);
        assert_eq!(id("minecraft:block_entity_type", "alpha:zblock"), 2);
        assert_eq!(id("minecraft:block_entity_type", "ruby:zblock"), 3);

        assert_eq!(id("minecraft:mob_effect", "minecraft:luck"), 25);
        assert_eq!(id("minecraft:mob_effect", "alpha:dull"), 26);
        assert_eq!(id("minecraft:mob_effect", "alpha:zap"), 27);
        assert_eq!(id("minecraft:mob_effect", "ruby:glow"), 28);

        // `/effect` and the mob_effect arguments check ids against this table.
        let mut commands: J = serde_json::from_str(
            r#"{"blocks":[],"items":[],"effects":["minecraft:speed"],
                "by_registry":{"minecraft:mob_effect":{"ids":["minecraft:speed"]}}}"#,
        )
        .unwrap();
        plan.json("command-registries.json", &mut commands).unwrap();
        let commands: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&commands).unwrap()).unwrap();
        let want = ["minecraft:speed", "alpha:dull", "alpha:zap", "ruby:glow"];
        assert_eq!(commands["effects"], serde_json::json!(want));
        assert_eq!(
            commands["by_registry"]["minecraft:mob_effect"]["ids"],
            serde_json::json!(want)
        );
        drop(plan);

        let output = scratch.join(format!("output-{index}"));
        build(&base, &mods, &output).unwrap();
        let built: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("registries.json")).unwrap()).unwrap();
        assert_eq!(
            built, registry,
            "the built root must use the tested allocation"
        );
        mods.reverse();
    }
    fs::remove_dir_all(scratch).unwrap();
}

fn prepared() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("../../.local/horizon-client/26.3"),
        manifest.join("../../../../.local/horizon-client/26.3"),
    ]
    .into_iter()
    .find(|root| root.join("gameplay-facts.json").exists())
}

/// A content mod folder: a full block, a door and an item-less glass from
/// templates, items, a recipe, a vanilla tag addition and a language file.
fn ruby_mod(dir: &Path) -> ContentMod {
    let files: [(&str, &str); 11] = [
        (
            "horizon-mod.toml",
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/ruby/horizon/block/ruby_block.json",
            r##"{"template": "minecraft:iron_block", "properties": {"hardness": 4.0, "light": 3,
                "sounds": "minecraft:amethyst_block", "map_color": 28, "friction": 0.8},
                "components": {"minecraft:max_stack_size": 16,
                    "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}}"##,
        ),
        (
            "data/ruby/horizon/block/ruby_door.json",
            r#"{"template": "minecraft:oak_door"}"#,
        ),
        (
            "data/ruby/horizon/block/ruby_glass.json",
            r#"{"template": "minecraft:glass", "item": false}"#,
        ),
        (
            "data/ruby/horizon/item/ruby.json",
            r#"{"template": "minecraft:diamond", "max_stack_size": 16, "rarity": "rare"}"#,
        ),
        (
            "data/ruby/horizon/item/ruby_sword.json",
            r#"{"template": "minecraft:iron_sword", "components": {"minecraft:max_damage": 900,
                "minecraft:attribute_modifiers": [{"type": "minecraft:attack_damage",
                    "id": "minecraft:base_attack_damage", "amount": 7.0,
                    "operation": "add_value", "slot": "mainhand"}],
                "!minecraft:repairable": {}}}"#,
        ),
        (
            "data/ruby/horizon/item/ruby_stew.json",
            r#"{"template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl"}"#,
        ),
        (
            "data/ruby/recipe/ruby_from_stew.json",
            r#"{"type": "minecraft:crafting_shapeless", "category": "misc",
                "ingredients": ["ruby:ruby_stew"], "result": {"id": "ruby:ruby"}}"#,
        ),
        // Carried-over tags name blocks left out and other mods' ids too.
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            r##"{"values": ["ruby:ruby_block", "ruby:left_out", "othermod:thing",
                {"id": "othermod:optional", "required": false}, "#c:storage_blocks/ruby"]}"##,
        ),
        (
            "data/c/tags/item/gems/ruby.json",
            r#"{"values": ["ruby:ruby", "othermod:ruby"]}"#,
        ),
        (
            "assets/ruby/lang/en_us.json",
            r#"{"block.ruby.ruby_block": "Block of Ruby"}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mods::Package::read(dir).unwrap();
    ContentMod {
        id: "ruby".into(),
        content: Content::read(dir, &package.manifest).unwrap(),
    }
}

/// The overlay over the real prepared resources: every loader reads it, the
/// new blocks behave like their templates with the overrides, and vanilla's
/// states are where they were.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_loads_with_every_table() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-content-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = ruby_mod(&scratch.join("ruby"));
    // The base as a relative path (cargo runs tests in the crate's folder):
    // the overlay's links still resolve.
    let relative = Path::new("../../.local/horizon-client/26.3");
    let root = resource_root(relative, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    assert_ne!(root, base);

    let vanilla = Assets::load(&base).unwrap();
    let assets = Assets::load(&root).unwrap();
    let (old, new) = (vanilla.registry, assets.registry);
    let n = old.len() as u32;
    let iron = old.default_state("minecraft:iron_block").unwrap();
    let door = old.default_state("minecraft:oak_door").unwrap();
    let door_first = (0..n)
        .map(BlockStateId)
        .find(|&s| old.state(s).unwrap().name == "minecraft:oak_door")
        .unwrap();
    // Vanilla keeps its ids; blocks come after, sorted: ruby_block, ruby_door, ruby_glass.
    assert_eq!(new.len(), old.len() + 1 + 64 + 1);
    assert_eq!(new.default_state("minecraft:iron_block"), Some(iron));
    let ruby = new.default_state("ruby:ruby_block").unwrap();
    assert_eq!(ruby, BlockStateId(n));
    let ruby_door = new.default_state("ruby:ruby_door").unwrap();
    assert_eq!(ruby_door, BlockStateId(n + 1 + (door.0 - door_first.0)));
    assert_eq!(
        new.default_state("ruby:ruby_glass"),
        Some(BlockStateId(n + 65))
    );
    // Network ids: the new items and blocks follow vanilla's in the registries report, which
    // the game and its server both read (a missing one travels as air).
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla_ids, ids) = (report(&base), report(&root));
    let item = |b: &horizon_protocol::registry::Builtin, name: &str| {
        b.get("minecraft:item").and_then(|r| r.id(name))
    };
    let vanilla_items = (0..)
        .take_while(|&i| vanilla_ids.get("minecraft:item").unwrap().name(i).is_some())
        .count() as i32;
    assert_eq!(
        item(&ids, "minecraft:stone"),
        item(&vanilla_ids, "minecraft:stone")
    );
    let ruby_item = item(&ids, "ruby:ruby_block").unwrap();
    assert!(ruby_item >= vanilla_items, "{ruby_item} < {vanilla_items}");
    assert_eq!(
        ids.get("minecraft:item").unwrap().name(ruby_item),
        Some("ruby:ruby_block")
    );
    assert!(item(&ids, "ruby:ruby_glass").is_none(), "no item declared");
    assert!(
        ids.get("minecraft:block")
            .unwrap()
            .id("ruby:ruby_glass")
            .is_some()
    );

    let old_facts = BlockFacts::load(&base, old).unwrap();
    let facts = BlockFacts::load(&root, new).unwrap();
    let shared_json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        fs::File::open(root.join("block-behaviours.json")).unwrap(),
    ))
    .unwrap();
    let (shared_facts, shared_behaviours) =
        BlockFacts::load_with_behaviours(&root, new, &shared_json).unwrap();
    let independent_behaviours = horizon_world::level::Behaviours::load(new, &root).unwrap();
    let (deferred_facts, gameplay) =
        BlockFacts::load_with_projection(&root, new, &shared_json, true).unwrap();
    let deferred_behaviours =
        horizon_world::level::Behaviours::load_with_gameplay(new, &root, &shared_json, &gameplay)
            .unwrap();
    assert_eq!(shared_facts.states.len(), facts.states.len());
    for state in [ruby, ruby_door, BlockStateId(n + 65)] {
        assert_eq!(shared_facts.get(state).hardness, facts.get(state).hardness);
        assert_eq!(
            deferred_facts.get(state).hardness,
            facts.get(state).hardness
        );
        assert_eq!(deferred_facts.get(state).classes, facts.get(state).classes);
        assert_eq!(
            deferred_behaviours.facts(state).flags,
            independent_behaviours.facts(state).flags
        );
        assert_eq!(
            deferred_behaviours.data(state).sounds.place,
            independent_behaviours.data(state).sounds.place
        );
        assert_eq!(shared_facts.get(state).emission, facts.get(state).emission);
        assert_eq!(shared_facts.get(state).classes, facts.get(state).classes);
        assert_eq!(
            shared_behaviours.facts(state).flags,
            independent_behaviours.facts(state).flags,
        );
        assert_eq!(
            shared_behaviours.data(state).sounds.place,
            independent_behaviours.data(state).sounds.place,
        );
    }

    let (a, b) = (old_facts.get(iron), facts.get(ruby));
    assert_eq!((b.hardness, b.emission, b.friction), (4.0, 3, 0.8));
    assert_eq!(b.explosion_resistance, a.explosion_resistance);
    assert_eq!(b.outline, a.outline);
    assert_eq!(b.classes, a.classes);
    assert_eq!(&*b.item, "ruby:ruby_block");
    assert_eq!(b.map_color, 28);
    let amethyst = old_facts.get(old.default_state("minecraft:amethyst_block").unwrap());
    assert_eq!(b.sounds.break_sound, amethyst.sounds.break_sound);
    for i in 0..64 {
        let (v, m) = (
            old_facts.get(BlockStateId(door_first.0 + i)),
            facts.get(BlockStateId(n + 1 + i)),
        );
        assert_eq!(
            (v.outline.clone(), v.hardness, v.classes.clone()),
            (m.outline.clone(), m.hardness, m.classes.clone())
        );
        assert_eq!(&*m.name, "ruby:ruby_door");
    }
    let glass = facts.get(BlockStateId(n + 65));
    assert_eq!(&*glass.item, "minecraft:air");
    // Vanilla's own rows are untouched.
    assert_eq!(facts.get(iron).hardness, a.hardness);
    assert_eq!(&*facts.get(iron).name, "minecraft:iron_block");

    let items = Items::load(&root, new, &facts, &PackOverlay::NONE).unwrap();
    let ruby_block = &items.items["ruby:ruby_block"];
    assert_eq!(ruby_block.block, Some(ruby));
    let iron_block = &items.items["minecraft:iron_block"];
    assert_eq!((ruby_block.max_stack, iron_block.max_stack), (16, 64));
    assert!(ruby_block.fire_resistant && !iron_block.fire_resistant);
    let gem = &items.items["ruby:ruby"];
    assert_eq!(gem.max_stack, 16);
    let sword = &items.items["ruby:ruby_sword"];
    let iron_sword = &items.items["minecraft:iron_sword"];
    assert_eq!((sword.max_damage, iron_sword.max_damage), (900, 250));
    assert_eq!(sword.modifiers.len(), 1);
    assert_eq!(sword.modifiers[0].2, 7.0);
    assert_eq!(sword.tool.len(), iron_sword.tool.len());
    // The stew leaves its bowl in the grid; vanilla's stew leaves none.
    let stew = &items.items["ruby:ruby_stew"];
    assert_eq!(stew.crafting_remainder.as_deref(), Some("minecraft:bowl"));
    assert_eq!(
        items.items["minecraft:mushroom_stew"].crafting_remainder,
        None
    );
    let recipes = crate::crafting::Recipes::load(&root, &items, &PackOverlay::NONE).unwrap();
    let grid = [
        Some(crate::items::ItemStack::new("ruby:ruby_stew", 1)),
        None,
        None,
        None,
    ];
    let (id, result) = recipes.find(&grid, 2).unwrap();
    assert_eq!(&*result.item, "ruby:ruby");
    let rest = recipes.remainders(id, &grid, 2);
    assert_eq!(rest[0].as_ref().map(|s| &*s.item), Some("minecraft:bowl"));
    assert!(!items.items.contains_key("ruby:ruby_glass"));
    let pickaxe = &items.block_tags["minecraft:mineable/pickaxe"];
    assert!(pickaxe.contains("ruby:ruby_block"));
    assert!(!pickaxe.contains("ruby:left_out") && !pickaxe.contains("othermod:thing"));
    assert!(pickaxe.contains("minecraft:stone"));
    // Convention tags load, with the ids nobody registers dropped.
    assert_eq!(items.item_tag_order["c:gems/ruby"], ["ruby:ruby"]);

    crate::collisions::load(&root, new).unwrap();
    crate::models::load_visibility(&root, new).unwrap();
    crate::light::LightTables::load(&root, new, &facts).unwrap();
    crate::motion::MotionResources::load(&root, &assets).unwrap();
    horizon_world::worldgen::generator::GeneratorData::load(&root.join("worldgen"), new).unwrap();
    assert!(root.join("assets/ruby/lang/en_us.json").is_file());
    // Blocks without a blockstate of their own take their template's.
    assert_eq!(
        fs::read(root.join("assets/ruby/blockstates/ruby_door.json")).unwrap(),
        fs::read(base.join("assets/minecraft/blockstates/oak_door.json")).unwrap()
    );

    // The base root was only read.
    assert!(
        fs::symlink_metadata(base.join("facts.json"))
            .unwrap()
            .is_file()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_unknown_templates_and_vanilla_ids() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-refuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    m.content.blocks[0].template = "minecraft:not_a_block".into();
    let e = build(&base, std::slice::from_ref(&m), &scratch.join("out")).unwrap_err();
    assert!(e.to_string().contains("is not a block"), "{e}");
    m.content.blocks[0].template = "minecraft:iron_block".into();
    let stew = m
        .content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby_stew");
    stew.unwrap().crafting_remainder = Some("minecraft:bowll".into());
    let e = build(&base, std::slice::from_ref(&m), &scratch.join("out")).unwrap_err();
    assert!(
        e.to_string()
            .contains(r#"crafting_remainder "minecraft:bowll" is not an item"#),
        "{e}"
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_more_states_than_a_world_holds() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-states-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // Redstone wire has 1296 states: 810 copies pass 2^20 with any game.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..810 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    let e = build(&base, std::slice::from_ref(&m), &scratch.join("out")).unwrap_err();
    assert!(
        e.to_string()
            .contains("more than the 1048576 a world can hold"),
        "{e}"
    );
    assert!(e.to_string().contains("the mods: ruby "), "{e}");
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn builds_past_sixteen_bit_states() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-wide-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // 40 redstone wires: 51,840 states, past 65,536 with vanilla's 35,723.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..40 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    build(&base, std::slice::from_ref(&m), &scratch.join("out")).unwrap();
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn no_content_keeps_the_base_root() {
    let base = Path::new("/nonexistent/base");
    assert_eq!(
        resource_root(base, &[], Path::new("/nonexistent/cache")).unwrap(),
        base
    );
}

use horizon_world::loot::PackOverlay;
use horizon_world::registry::BlockStateId;

#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn shared_behaviour_document_matches_independent_loaders() {
    let Some(root) = prepared() else { return };
    let assets = Assets::load(&root).expect("assets");
    let doc: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(root.join("block-behaviours.json")).unwrap(),
    ))
    .unwrap();
    let original = BlockFacts::load(&root, assets.registry).unwrap();
    let (shared, b) = BlockFacts::load_with_behaviours(&root, assets.registry, &doc).unwrap();
    let (deferred_facts, projection) =
        BlockFacts::load_with_projection(&root, assets.registry, &doc, true).unwrap();
    let deferred = horizon_world::level::Behaviours::load_with_gameplay(
        assets.registry,
        &root,
        &doc,
        &projection,
    )
    .unwrap();
    assert_eq!(deferred_facts.states.len(), shared.states.len());
    assert_eq!(original.states.len(), shared.states.len());
    for (a, b) in original.states.iter().zip(&shared.states) {
        assert_eq!(a.classes, b.classes);
        assert_eq!(a.legacy_solid, b.legacy_solid);
        assert_eq!(a.sturdy_full, b.sturdy_full);
        assert_eq!(a.emission, b.emission);
        assert_eq!(a.dampening, b.dampening);
        assert_eq!(a.hardness, b.hardness);
        assert_eq!(a.sounds.volume, b.sounds.volume);
        assert_eq!(a.sounds.pitch, b.sounds.pitch);
    }
    let a = horizon_world::level::Behaviours::load(assets.registry, &root).unwrap();
    for id in 0..assets.registry.len() {
        let state = horizon_world::registry::BlockStateId(id as u32);
        let (a_state, b_state) = (a.facts(state), b.facts(state));
        let deferred_state = deferred.facts(state);
        assert_eq!(b_state.flags, deferred_state.flags);
        assert_eq!(b_state.emission, deferred_state.emission);
        assert_eq!(b_state.dampening, deferred_state.dampening);
        assert_eq!(b_state.hardness, deferred_state.hardness);
        assert_eq!(b_state.push, deferred_state.push);
        assert_eq!(a_state.flags, b_state.flags);
        assert_eq!(a_state.sturdy, b_state.sturdy);
        assert_eq!(a_state.weak, b_state.weak);
        assert_eq!(a_state.strong, b_state.strong);
        assert_eq!(a_state.fluid, b_state.fluid);
        assert_eq!(a_state.emission, b_state.emission);
        assert_eq!(a_state.dampening, b_state.dampening);
        assert_eq!(a_state.hardness, b_state.hardness);
        assert_eq!(a_state.push, b_state.push);
        assert_eq!(a_state.instrument, b_state.instrument);
        assert_eq!(a_state.shape, b_state.shape);
        let (a_sound, b_sound) = (&a.data(state).sounds, &b.data(state).sounds);
        assert_eq!(a_sound.volume, b_sound.volume);
        assert_eq!(a_sound.pitch, b_sound.pitch);
        assert_eq!(a_sound.place, b_sound.place);
        assert_eq!(a_sound.break_, b_sound.break_);
        assert_eq!(a_sound.fall, b_sound.fall);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn independent_reloads_read_changed_behaviour_json_and_report_invalid_files() {
    let Some(base) = prepared() else { return };
    let scratch = std::env::temp_dir().join(format!("horizon-json-reloads-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    // Keep the remaining resource tree intact without copying a huge prepared client.
    for entry in fs::read_dir(&base).unwrap() {
        let entry = entry.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(entry.path(), scratch.join(entry.file_name())).unwrap();
    }
    let assets = Assets::load(&base).unwrap();
    let registry = assets.registry;
    let original = fs::read(base.join("block-behaviours.json")).unwrap();
    fs::remove_file(scratch.join("block-behaviours.json")).unwrap();
    fs::write(scratch.join("block-behaviours.json"), &original).unwrap();
    // Independent facts and behaviours can load first, followed by paired data.
    let independent_first = BlockFacts::load(&scratch, registry).unwrap();
    let independent_world_first =
        horizon_world::level::Behaviours::load(registry, &scratch).unwrap();
    let (facts, paired_world) = BlockFacts::load_pair(&scratch, registry).unwrap();
    assert_eq!(facts.states.len(), registry.len());
    for state in [0, 1, registry.len() - 1] {
        let state = horizon_world::registry::BlockStateId(state as u32);
        assert_eq!(
            facts.get(state).classes,
            independent_first.get(state).classes
        );
        assert_eq!(
            facts.get(state).sounds.place,
            independent_first.get(state).sounds.place
        );
        assert_eq!(
            paired_world.facts(state).flags,
            independent_world_first.facts(state).flags
        );
    }

    // Override one exported state bit at the same path and verify that a later
    // independent and paired read sees it while the original resource root does not.
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let original_bits = changed["states"][0][1].as_u64().unwrap();
    changed["states"][0][1] = (original_bits ^ 1).into();
    fs::write(
        scratch.join("block-behaviours.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let changed_pair = BlockFacts::load_pair(&scratch, registry).unwrap();
    let changed_independent = BlockFacts::load(&scratch, registry).unwrap();
    let base_facts = BlockFacts::load(&base, registry).unwrap();
    let state = horizon_world::registry::BlockStateId(0);
    assert_ne!(
        changed_pair.0.get(state).legacy_solid,
        base_facts.get(state).legacy_solid
    );
    assert_eq!(
        changed_pair.0.get(state).legacy_solid,
        changed_independent.get(state).legacy_solid
    );
    // The world loader sees the same overridden legacy solidity flag.
    let changed_world = horizon_world::level::Behaviours::load(registry, &scratch).unwrap();
    assert_eq!(
        changed_world.facts(state).flags & 1 != 0,
        changed_pair.0.get(state).legacy_solid
    );

    fs::write(scratch.join("block-behaviours.json"), b"{").unwrap();
    let paired_error = BlockFacts::load_pair(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    let independent_error = BlockFacts::load(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(paired_error, independent_error);
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), b"{}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), original).unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_file(scratch.join("gameplay-facts.json")).unwrap();
    fs::write(scratch.join("gameplay-facts.json"), b"{\"states\":[]}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(scratch.join("gameplay-facts.json"), b"{").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(
        scratch.join("gameplay-facts.json"),
        fs::read(base.join("gameplay-facts.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// A built root is kept for later starts, and built again when a mod's
/// files change.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_is_reused_until_a_mod_changes() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-reuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = ruby_mod(&scratch.join("ruby"));
    let cache = scratch.join("cache");
    let root = resource_root(&base, std::slice::from_ref(&m), &cache).unwrap();
    // A rebuild starts from an empty folder: this file tells them apart.
    let sentinel = root.join("sentinel");
    fs::write(&sentinel, b"").unwrap();
    assert_eq!(
        resource_root(&base, std::slice::from_ref(&m), &cache).unwrap(),
        root
    );
    assert!(sentinel.exists(), "an unchanged mod rebuilt the root");
    let lang = scratch.join("ruby/assets/ruby/lang/en_us.json");
    fs::write(&lang, r#"{"block.ruby.ruby_block": "Ruby Block"}"#).unwrap();
    let file = fs::File::options().write(true).open(&lang).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    resource_root(&base, std::slice::from_ref(&m), &cache).unwrap();
    assert!(!sentinel.exists(), "a changed mod kept the old root");
    assert!(
        fs::read_to_string(root.join("assets/ruby/lang/en_us.json"))
            .unwrap()
            .contains("Ruby Block")
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A structure mod's worldgen reaches the worldgen loader: its structure,
/// set, pool and templates join the bundles, and its biome tag keeps only
/// what the loader knows.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_carries_a_structure_mod() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-worldgen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("huts");
    let files = [
        (
            "horizon-mod.toml",
            "id = \"huts\"\nname = \"Huts\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/huts/worldgen/structure/hut.json",
            r##"{"type": "minecraft:jigsaw", "biomes": "#huts:has_hut", "step": "surface_structures",
                "spawn_overrides": {}, "terrain_adaptation": "beard_thin", "start_pool": "huts:hut/start",
                "size": 1, "start_height": {"absolute": 0}, "project_start_to_heightmap": "WORLD_SURFACE_WG",
                "max_distance_from_center": 80}"##,
        ),
        (
            "data/huts/worldgen/template_pool/hut/start.json",
            r#"{"fallback": "minecraft:empty", "elements": [{"weight": 1, "element": {
                "element_type": "minecraft:single_pool_element", "location": "huts:hut",
                "projection": "rigid", "processors": "minecraft:empty"}}]}"#,
        ),
        (
            "data/huts/worldgen/structure_set/huts.json",
            r#"{"structures": [{"structure": "huts:hut", "weight": 1}],
                "placement": {"type": "minecraft:random_spread", "salt": 7, "spacing": 4, "separation": 2}}"#,
        ),
        (
            "data/huts/tags/worldgen/biome/has_hut.json",
            r##"{"values": ["#minecraft:is_overworld", "#c:is_plains", "othermod:glade", "minecraft:plains"]}"##,
        ),
        // Additions to vanilla's set, pool and processor list; a target nobody
        // defines is skipped. A lower priority goes first.
        (
            "data/huts/worldgen/structure_set/added/villages.json",
            r#"{"horizon:add_to": "minecraft:villages",
                "structures": [{"structure": "huts:hut", "weight": 3}]}"#,
        ),
        (
            "data/huts/worldgen/template_pool/added/houses.json",
            r#"{"horizon:add_to": ["othermod:houses", "minecraft:village/plains/houses"],
                "elements": [{"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}]}"#,
        ),
        (
            "data/huts/worldgen/template_pool/added/more_houses.json",
            r#"{"horizon:add_to": "minecraft:village/plains/houses", "horizon:priority": 10,
                "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}]}"#,
        ),
        (
            "data/huts/worldgen/processor_list/added/mossy.json",
            r#"{"horizon:add_to": "minecraft:mossify_10_percent",
                "processors": [{"processor_type": "minecraft:nop"}]}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    // The template: vanilla's smallest plains house, as the mod's own file.
    let house =
        base.join("worldgen/structure/minecraft/village/plains/houses/plains_small_house_1.nbt");
    fs::create_dir_all(dir.join("data/huts/structure")).unwrap();
    fs::copy(&house, dir.join("data/huts/structure/hut.nbt")).unwrap();
    let package = horizon_mods::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "huts".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
    };
    let root = resource_root(&base, &[m], &scratch.join("cache")).unwrap();
    assert!(root.join("worldgen/structure/huts/hut.nbt").exists());
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    let hut: serde_json::Value = serde_json::from_str(&tags["huts:has_hut"]).unwrap();
    assert_eq!(
        hut["values"],
        serde_json::json!(["#minecraft:is_overworld", "minecraft:plains"])
    );
    // The additions join vanilla's entries at the end and are no set or pool
    // of their own.
    let bundle = |name: &str, base: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(base.join("worldgen").join(name)).unwrap()).unwrap()
    };
    for (name, id, list, added) in [
        (
            "structure_sets.json",
            "minecraft:villages",
            "structures",
            vec![serde_json::json!({"structure": "huts:hut", "weight": 3})],
        ),
        (
            "template_pools.json",
            "minecraft:village/plains/houses",
            "elements",
            vec![
                serde_json::json!({"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}),
                serde_json::json!({"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}),
            ],
        ),
        (
            "processor_lists.json",
            "minecraft:mossify_10_percent",
            "processors",
            vec![serde_json::json!({"processor_type": "minecraft:nop"})],
        ),
    ] {
        let (ours, vanilla) = (bundle(name, &root), bundle(name, &base));
        let entries = |text: &str| -> Vec<serde_json::Value> {
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            v[list].as_array().unwrap().clone()
        };
        let mut expected = entries(&vanilla[id]);
        expected.extend(added);
        assert_eq!(entries(&ours[id]), expected, "{id}");
        assert!(!ours.keys().any(|k| k.starts_with("huts:added/")), "{name}");
        assert!(!ours.contains_key("othermod:houses"));
    }

    // The commands suggest the mod's structure, set and pool.
    let registries: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for (registry, id) in [
        ("minecraft:worldgen/structure", "huts:hut"),
        ("minecraft:worldgen/structure_set", "huts:huts"),
        ("minecraft:worldgen/template_pool", "huts:hut/start"),
    ] {
        let ids = registries["by_registry"][registry]["ids"]
            .as_array()
            .unwrap();
        assert!(ids.iter().any(|v| v == id), "{registry} lacks {id}");
        assert!(
            ids.iter()
                .any(|v| v.as_str().unwrap().starts_with("minecraft:"))
        );
    }

    let assets = Assets::load(&root).unwrap();
    let biomes = crate::biome::Biomes::load(&root).unwrap();
    let terrain = crate::vanillagen::VanillaTerrain::new(
        &root.join("worldgen"),
        assets.registry,
        &biomes,
        1234,
    )
    .unwrap();
    let (name, at) = terrain
        .locate(&["huts:hut".to_owned()], [0, 64, 0], 16)
        .expect("the mod's structure generates");
    assert_eq!(name, "huts:hut");
    assert!(at[0].abs() < 1024 && at[2].abs() < 1024, "{at:?}");
    let _ = fs::remove_dir_all(&scratch);
}

/// `FlatLevelSource.createState`: a superflat world runs the structure sets its
/// `structure_overrides` allow (the classic flat world: strongholds and villages) on its
/// flat ground, and no other set, with the layers where the terrain would be.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn a_superflat_world_runs_its_structure_sets_on_flat_ground() {
    use crate::streaming::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let flat = horizon_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:grass_block".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec![
            "minecraft:strongholds".into(),
            "minecraft:villages".into(),
        ]),
    };
    let terrain = crate::vanillagen::VanillaTerrain::for_flat(
        &base.join("worldgen"),
        &Default::default(),
        assets.registry,
        &biomes,
        1234,
        &flat,
        true,
    )
    .unwrap();
    // A village and a stronghold are found; a mineshaft is not in the overrides.
    let village = terrain.locate(&["minecraft:village_plains".to_owned()], [0, 64, 0], 12);
    assert!(village.is_some(), "no plains village near the origin");
    assert!(
        terrain
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
    assert!(
        terrain
            .locate(&["minecraft:mineshaft".to_owned()], [0, 64, 0], 12)
            .is_none()
    );
    // The layers: bedrock, two dirt and grass from the level's minimum Y, air above.
    let mut worker = terrain.worker();
    let chunk = terrain.finish(terrain.proto(&mut worker, 3, 3));
    let name = |y| {
        let id = chunk.block(3 * 16 + 5, y, 3 * 16 + 7);
        assets.registry.state(id).unwrap().name.clone()
    };
    assert_eq!(
        [-65, -64, -63, -62, -61, -60].map(name),
        [
            "minecraft:air",
            "minecraft:bedrock",
            "minecraft:dirt",
            "minecraft:dirt",
            "minecraft:grass_block",
            "minecraft:air"
        ]
        .map(String::from)
    );
}

/// `createForFlat` searches the stronghold rings with seed 0, so a superflat world's
/// strongholds are in the same place on every seed; a layer that is not opaque to
/// `MOTION_BLOCKING` is `null`, left out of `buildTerrain` for the fill after the structures.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn a_superflat_world_keeps_its_strongholds_on_every_seed_and_leaves_null_layers_for_the_fill() {
    use crate::streaming::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let flat = horizon_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:snow".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec!["minecraft:strongholds".into()]),
    };
    let make = |seed| {
        crate::vanillagen::VanillaTerrain::for_flat(
            &base.join("worldgen"),
            &Default::default(),
            assets.registry,
            &biomes,
            seed,
            &flat,
            true,
        )
        .unwrap()
    };
    let stronghold = |seed| {
        make(seed)
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 100)
            .expect("a stronghold")
    };
    assert_eq!(stronghold(1234), stronghold(-987654321));
    // `moveBelowSeaLevel(-63, ChunkGenerator.getMinY() = 0, ..)`: the sea level less 10 is far
    // below the room's height above 0, so the stronghold's top lands at `ySpan + 1` above Y 0
    // (with the level's -64 it would sit 64 blocks lower). The start's box is inflated by 12
    // for the terrain adaptation: the pieces span Y 2 to 36, the box -10 to 48.
    let (_, at) = stronghold(1234);
    let starts = make(1234).starts_at(at[0] >> 4, at[2] >> 4);
    let start = starts
        .iter()
        .find(|s| s.name == "minecraft:stronghold")
        .expect("the stronghold start");
    assert!(
        start.bounding_box.min[1] >= -12,
        "the stronghold sits below the generator's Y 0: {:?}",
        start.bounding_box
    );
    let terrain = make(1234);
    let mut worker = terrain.worker();
    let chunk = terrain.finish(terrain.proto(&mut worker, 3, 3));
    let name = |y| {
        let id = chunk.block(3 * 16 + 5, y, 3 * 16 + 7);
        assets.registry.state(id).unwrap().name.clone()
    };
    // The snow layer (index 3, y -61) is filled later, not by the terrain.
    assert_eq!(
        [-64, -63, -62, -61].map(name),
        [
            "minecraft:bedrock",
            "minecraft:dirt",
            "minecraft:dirt",
            "minecraft:air"
        ]
        .map(String::from)
    );
}

/// `structure_overrides` is a lenient field: a list naming a structure set that does not exist
/// fails to decode and counts as absent, so every set generates.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn a_superflat_structure_override_naming_no_set_lets_every_set_generate() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let flat = horizon_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:grass_block".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec!["minecraft:village".into()]),
    };
    let terrain = crate::vanillagen::VanillaTerrain::for_flat(
        &base.join("worldgen"),
        &Default::default(),
        assets.registry,
        &biomes,
        1234,
        &flat,
        true,
    )
    .unwrap();
    assert!(
        terrain
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
    assert!(
        terrain
            .locate(&["minecraft:village_plains".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
}

/// A mod's mob effect gets the network id after vanilla's last, on the game
/// and its server alike, and joins the ids `/effect` and its suggestions
/// check.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_registers_a_mod_mob_effect() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-effect-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("glow"),
        &[
            ("horizon-mod.toml", &manifest("glow", "")),
            (
                "data/glow/horizon/mob_effect/radiance.json",
                r#"{"category": "beneficial", "color": 16773120}"#,
            ),
        ],
    );
    let root = resource_root(&base, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:mob_effect").unwrap();
    let ids = ids.get("minecraft:mob_effect").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("glow:radiance"), Some(count));
    assert_eq!(ids.id("minecraft:speed"), vanilla.id("minecraft:speed"));
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for ids in [
        &commands["effects"],
        &commands["by_registry"]["minecraft:mob_effect"]["ids"],
    ] {
        let ids = ids.as_array().unwrap();
        assert!(ids.contains(&"glow:radiance".into()) && ids.contains(&"minecraft:speed".into()));
    }
    assert!(root.join(crate::content_identity::MARKER).exists());
    fs::remove_dir_all(&scratch).unwrap();
}

/// A mod that adds only an item still moves the item network ids, so a game
/// without it must fail the content check rather than show other items.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_with_only_an_item_checks_its_identity() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("gem"),
        &[
            ("horizon-mod.toml", &manifest("gem", "")),
            (
                "data/gem/horizon/item/opal.json",
                r#"{"template": "minecraft:diamond"}"#,
            ),
        ],
    );
    let root = resource_root(&base, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let ids = horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap();
    assert!(ids.get("minecraft:item").unwrap().id("gem:opal").is_some());
    assert!(root.join(crate::content_identity::MARKER).exists());
    assert_ne!(
        crate::content_identity::identity(&root).unwrap(),
        crate::content_identity::identity(&base).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// a mod's thrown entity type gets the network id after vanilla's last, the
/// commands accept it, and it changes the content identity.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_registers_a_mod_entity_type() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-entity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("example"),
        &[
            ("horizon-mod.toml", &manifest("example", "")),
            (
                "data/example/horizon/item/gem.json",
                r#"{"template": "minecraft:snowball"}"#,
            ),
            (
                "data/example/horizon/entity_type/bolt.json",
                r#"{"template": "minecraft:snowball", "item": "example:gem", "damage": 2}"#,
            ),
        ],
    );
    let root = resource_root(&base, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:entity_type").unwrap();
    let ids = ids.get("minecraft:entity_type").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("example:bolt"), Some(count));
    assert_eq!(
        ids.id("minecraft:snowball"),
        vanilla.id("minecraft:snowball")
    );
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for ids in [
        &commands["entity_types"],
        &commands["by_registry"]["minecraft:entity_type"]["ids"],
    ] {
        assert!(ids.as_array().unwrap().contains(&"example:bolt".into()));
    }
    // its size and eyes are the snowball's, for `execute anchored eyes` and spawners.
    let types: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("entity-types.json")).unwrap()).unwrap();
    assert!(types["minecraft:snowball"].is_object());
    assert_eq!(types["example:bolt"], types["minecraft:snowball"]);
    // the rewritten table keeps vanilla's floats to the bit (a zombie's speed).
    let speed = |root: &Path| {
        let attributes =
            crate::mobs::exact_attributes(&fs::read(root.join("entity-types.json")).unwrap());
        attributes["minecraft:zombie"]["minecraft:movement_speed"]
    };
    assert_eq!(speed(&root).to_bits(), speed(&base).to_bits());
    assert!(root.join(crate::content_identity::MARKER).exists());
    let identity = crate::content_identity::identity;
    assert_ne!(identity(&root).unwrap(), identity(&base).unwrap());
    fs::remove_dir_all(&scratch).unwrap();
}

/// a mod's menu type gets the menu network id after vanilla's last, and it
/// changes the content identity.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_registers_a_mod_menu_type() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-menu-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("example"),
        &[
            ("horizon-mod.toml", &manifest("example", "")),
            (
                "data/example/horizon/menu/press.json",
                r#"{"slots": 9, "data_slots": 2, "results": [7, 8]}"#,
            ),
        ],
    );
    let root = resource_root(&base, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:menu").unwrap();
    let ids = ids.get("minecraft:menu").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("example:press"), Some(count));
    assert_eq!(ids.id("minecraft:furnace"), vanilla.id("minecraft:furnace"));
    assert!(root.join(crate::content_identity::MARKER).exists());
    let identity = crate::content_identity::identity;
    assert_ne!(identity(&root).unwrap(), identity(&base).unwrap());
    fs::remove_dir_all(&scratch).unwrap();
}

/// A content mod folder from `(path, text)` pairs, read as the game reads it.
fn folder_mod(dir: &Path, files: &[(&str, &str)]) -> ContentMod {
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mods::Package::read(dir).unwrap();
    ContentMod {
        id: package.manifest.id.clone(),
        content: Content::read(dir, &package.manifest).unwrap(),
    }
}

fn manifest(id: &str, namespaces: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\nnamespaces = [{namespaces}]\n"
    )
}

/// A mod whose files sit under namespaces besides its id (Towns and Towers'
/// `kaisyn`): they join the root, and loaders read those namespaces.
#[test]
fn listed_namespaces_join_the_root() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-ns-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let toml = manifest("towers", "\"kaisyn\"");
    let m = folder_mod(
        &scratch.join("towers"),
        &[
            ("horizon-mod.toml", &toml),
            (
                "data/kaisyn/worldgen/processor_list/mossy.json",
                r#"{"processors": []}"#,
            ),
            (
                "data/kaisyn/tags/worldgen/biome/has_tower.json",
                r#"{"values": ["minecraft:plains"]}"#,
            ),
            (
                "assets/kaisyn/lang/en_us.json",
                r#"{"block.kaisyn.x": "X"}"#,
            ),
        ],
    );
    let root = scratch.join("root");
    build(&base, &[m], &root).unwrap();
    assert_eq!(namespaces(&root), ["kaisyn", "towers"]);
    assert!(tag_namespaces(&root).contains(&"kaisyn".to_owned()));
    let lists: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/processor_lists.json")).unwrap())
            .unwrap();
    assert!(lists.contains_key("kaisyn:mossy"));
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    assert!(tags.contains_key("kaisyn:has_tower"));
    let _ = fs::remove_dir_all(&scratch);
}

/// Two mods owning one namespace (one's listed namespace is the other's id,
/// or both list it) are refused together, naming both.
#[test]
fn mods_sharing_a_namespace_are_refused() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-clash-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let towers = folder_mod(
        &scratch.join("towers"),
        &[("horizon-mod.toml", &manifest("towers", "\"kaisyn\""))],
    );
    let kaisyn = folder_mod(
        &scratch.join("kaisyn"),
        &[("horizon-mod.toml", &manifest("kaisyn", ""))],
    );
    let other = folder_mod(
        &scratch.join("other"),
        &[("horizon-mod.toml", &manifest("other", "\"kaisyn\""))],
    );
    // The check runs before the base is read.
    let base = scratch.join("no-base");
    let out = scratch.join("root");
    let e = build(&base, &[towers, kaisyn], &out).unwrap_err();
    assert_eq!(
        e.to_string(),
        "mods towers and kaisyn both use the namespace kaisyn; turn one of them off"
    );
    let towers = folder_mod(
        &scratch.join("towers"),
        &[("horizon-mod.toml", &manifest("towers", "\"kaisyn\""))],
    );
    let e = build(&base, &[towers, other], &out).unwrap_err();
    assert!(e.to_string().starts_with("mods towers and other"), "{e}");
    let _ = fs::remove_dir_all(&scratch);
}

/// Mods merge into a tag in load order, as data packs do: each adds its
/// values, and one that replaces drops what came before it, with a warning
/// naming what it dropped.
#[test]
fn tags_merge_in_load_order_and_replace_warns() {
    let v = |ids: &[&str]| -> Vec<J> { ids.iter().map(|i| J::Str((*i).into())).collect() };
    let file = "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json";
    let mut merge = TagMerge::default();
    assert_eq!(merge.add("a", file, false, true, &v(&["a:x"])), None);
    let warning = merge.add("b", file, true, true, &v(&["minecraft:plains"]));
    assert_eq!(
        warning.as_deref(),
        Some(
            "mod b replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: \
             vanilla's entries and those of a are left out, as a data pack's replace does; \
             the mods after it in load order still add to it"
        )
    );
    assert_eq!(merge.add("c", file, false, true, &v(&["c:y"])), None);
    assert!(merge.replaced);
    assert_eq!(merge.values, v(&["minecraft:plains", "c:y"]));
    // A second replace drops only the mods' entries.
    let warning = merge.add("d", file, true, true, &v(&["d:z"])).unwrap();
    assert!(
        warning.contains(": those of b, c are left out"),
        "{warning}"
    );
    assert_eq!(merge.values, v(&["d:z"]));
    // A mod replacing a tag of its own that nobody added to drops nothing.
    let own = "data/a/tags/item/gems.json";
    assert_eq!(
        TagMerge::default().add("a", own, true, false, &v(&["a:x"])),
        None
    );
}

/// A replacing mod's tag leaves vanilla's entries out of the tag file and of
/// the worldgen loader's biome tags, and the root keeps the warning for the
/// starts that reuse it.
#[test]
fn a_replacing_mod_tag_reaches_the_root() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-replace-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let file = "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json";
    let adds = folder_mod(
        &scratch.join("adds"),
        &[
            ("horizon-mod.toml", &manifest("adds", "")),
            (file, r#"{"values": ["minecraft:forest"]}"#),
        ],
    );
    let towns = folder_mod(
        &scratch.join("towns"),
        &[
            ("horizon-mod.toml", &manifest("towns", "")),
            (file, r#"{"replace": true, "values": ["minecraft:plains"]}"#),
        ],
    );
    let late = folder_mod(
        &scratch.join("late"),
        &[
            ("horizon-mod.toml", &manifest("late", "")),
            (file, r#"{"values": ["minecraft:savanna"]}"#),
        ],
    );
    let vanilla: serde_json::Value =
        serde_json::from_slice(&fs::read(base.join(file)).unwrap()).unwrap();
    assert!(vanilla["values"].as_array().unwrap().len() > 1, "{vanilla}");
    let root = resource_root(&base, &[adds, towns, late], &scratch.join("cache")).unwrap();
    let expected = serde_json::json!(["minecraft:plains", "minecraft:savanna"]);
    let tag: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap();
    assert_eq!(tag["values"], expected);
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    let bundled: serde_json::Value =
        serde_json::from_str(&tags["minecraft:has_structure/village_plains"]).unwrap();
    assert_eq!(bundled["values"], expected);
    let warnings = fs::read_to_string(root.join(WARNINGS)).unwrap();
    assert!(
        warnings.starts_with("mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: vanilla's entries and those of adds are left out"),
        "{warnings}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// `StructureCheck.onStructureLoad`: the references a chunk saved for its start
/// count, so a map skips a start a vanilla map already pointed at.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn exploration_maps_skip_the_starts_a_chunk_saved_as_referenced() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let terrain = crate::vanillagen::VanillaTerrain::new(
        &base.join("worldgen"),
        assets.registry,
        &biomes,
        1234,
    )
    .unwrap();
    let names = ["minecraft:buried_treasure".to_owned()];
    let first = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    // Another terrain, on the same seed, whose chunks saved one reference for it.
    let fresh = crate::vanillagen::VanillaTerrain::new(
        &base.join("worldgen"),
        assets.registry,
        &biomes,
        1234,
    )
    .unwrap();
    let (name, at) = (first.0.clone(), (first.1[0] >> 4, first.1[2] >> 4));
    fresh.set_saved_references(std::sync::Arc::new(move |n, x, z| {
        i32::from(n == name && (x, z) == at)
    }));
    let next = fresh.locate_for_map(&names, [0, 64, 0], 50, true).unwrap();
    assert_ne!(next.1, first.1, "the saved reference is skipped");
    assert_eq!(
        fresh.locate_for_map(&names, [0, 64, 0], 50, false).unwrap(),
        first,
        "a map that does not skip still finds it"
    );
}

/// `ChunkGenerator.findNearestMapStructure` with `createReference`: an
/// exploration map that skips known structures points at a start no map
/// pointed at before, and a reopened world remembers which.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn exploration_maps_skip_the_structures_maps_pointed_at() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-map-refs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let open = || {
        let terrain = crate::vanillagen::VanillaTerrain::new(
            &base.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
        )
        .unwrap();
        terrain.set_reference_file(scratch.join("refs.json"));
        terrain
    };
    let names = ["minecraft:buried_treasure".to_owned()];
    let terrain = open();
    let first = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert_eq!(
        terrain.locate(&names, [0, 64, 0], 50).unwrap(),
        first,
        "/locate still finds the nearest"
    );
    let second = terrain
        .locate_for_map(&names, [0, 64, 0], 50, true)
        .unwrap();
    assert_ne!(second.1, first.1, "the second map points elsewhere");
    let again = terrain
        .locate_for_map(&names, [0, 64, 0], 50, false)
        .unwrap();
    assert_eq!(
        again, first,
        "without skipping the nearest is still the first"
    );
    drop(terrain);
    let third = open().locate_for_map(&names, [0, 64, 0], 50, true).unwrap();
    assert!(
        third.1 != first.1 && third.1 != second.1,
        "a reopened world remembers {first:?} and {second:?}, found {third:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// `NoiseBasedChunkGenerator.addDebugScreenInfo`: F3's `Density` line lists
/// the noise settings' `debug_functions` with three decimals.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn debug_density_line_lists_the_noise_settings_functions() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let terrain = crate::vanillagen::VanillaTerrain::new(
        &base.join("worldgen"),
        assets.registry,
        &biomes,
        1234,
    )
    .unwrap();
    let line = terrain.density_debug_line([0, 70, 0]).unwrap();
    let labels: Vec<&str> = line
        .strip_prefix("Density ")
        .unwrap()
        .split(' ')
        .step_by(2)
        .collect();
    assert_eq!(
        labels,
        ["N:", "T:", "V:", "C:", "E:", "D:", "W:", "PV:", "PS:"],
        "{line}"
    );
    assert!(
        line.split(' ')
            .skip(2)
            .step_by(2)
            .all(|v| v.split('.').nth(1).is_some_and(|d| d.len() == 3)),
        "{line}"
    );
}

#[test]
#[ignore = "full content root: needs prepared vanilla data"]
fn content_shapes_drive_outline_and_collision() {
    use horizon_core::{
        geometry::{Aabb, Axis, Vec3},
        position::BlockPos,
    };
    use horizon_world::{collision::CachedCollisionRegistry, registry::BlockStateId};
    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("horizon-content-shapes-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("horizon-mod.toml", &manifest("shapetest", "")),
            (
                "data/shapetest/horizon/block/rod.json",
                r#"{"template":"minecraft:oak_log","shapes":{"property":"axis","values":{"x":[[0,0.25,0.25,1,0.75,0.75]],"y":[[0.25,0,0.25,0.75,1,0.75]],"z":[[0.25,0.25,0,0.75,0.75,1]]}}}"#,
            ),
            (
                "data/shapetest/horizon/block/plain.json",
                r#"{"template":"minecraft:oak_log"}"#,
            ),
        ],
    );
    let root = resource_root(&base, &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let (named, _) = nbt::read_named(
        &fs::read(root.join("cached-collision.nbt")).unwrap(),
        Limits::tables(),
    )
    .unwrap();
    let collision = CachedCollisionRegistry::from_nbt(&named.tag, registry).unwrap();
    for axis in ["x", "y", "z"] {
        let state = (0..registry.len())
            .map(|i| BlockStateId(i as u32))
            .find(|id| {
                let s = registry.state(*id).unwrap();
                s.name == "shapetest:rod" && s.properties.get("axis").is_some_and(|v| v == axis)
            })
            .unwrap();
        let shape = collision.cached_shape(state).unwrap().unwrap();
        let f = facts.get(state);
        assert_eq!(shape.boxes().len(), 1);
        assert_eq!(f.outline.len(), 1);
        assert_eq!(f.outline_edges.len(), 12);
        let expected = match axis {
            "x" => [0., 0.25, 0.25, 1., 0.75, 0.75],
            "y" => [0.25, 0., 0.25, 0.75, 1., 0.75],
            _ => [0.25, 0.25, 0., 0.75, 0.75, 1.],
        };
        assert_eq!(f.outline[0], expected);
        let start = Vec3::new(-1., 0.5, 0.5);
        let end = Vec3::new(2., 0.5, 0.5);
        let target = BlockPos::new(0, 0, 0);
        assert!(shape.clip(start, end, target).is_some());
        let bounds = f.outline[0];
        let outline = Aabb::new(
            Vec3::new(bounds[0], bounds[1], bounds[2]),
            Vec3::new(bounds[3], bounds[4], bounds[5]),
        );
        assert_eq!(
            shape.clip(start, end, target),
            Aabb::clip_boxes([outline], start, end, target)
        );
        let player = Aabb::new(Vec3::new(-1., 0.4, 0.4), Vec3::new(-0.2, 0.6, 0.6));
        assert!((shape.collide(Axis::X, player, 2.) - (expected[0] + 0.2)).abs() < 1e-9);
        let miss = Aabb::new(Vec3::new(-1., 0.05, 0.05), Vec3::new(-0.2, 0.15, 0.15));
        assert_eq!(shape.collide(Axis::X, miss, 2.), 2.);
    }
    let plain = registry.default_state("shapetest:plain").unwrap();
    let oak = registry.default_state("minecraft:oak_log").unwrap();
    assert_eq!(facts.get(plain).outline, facts.get(oak).outline);
    assert_eq!(
        collision.cached_shape(plain).unwrap().unwrap().boxes(),
        collision.cached_shape(oak).unwrap().unwrap().boxes()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn content_block_default_state_is_validated_and_overlaid() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-default-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    let facts = r#"{"states":[
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"up","extended":"false"}}],
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"false"}}],
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"true"}}]],
      "defaults":{"minecraft:piston":0}}"#;
    fs::write(base.join("facts.json"), facts).unwrap();
    fs::write(base.join("gameplay-facts.json"), r#"{"states":[{"item":"minecraft:piston"},{"item":"minecraft:piston"},{"item":"minecraft:piston"}]}"#).unwrap();
    fs::write(base.join("items.json"), r#"{"minecraft:piston":{}}"#).unwrap();
    fs::write(base.join("registries.json"), r#"{"minecraft:item":{"entries":{"minecraft:piston":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:piston":{"protocol_id":0}}}}"#).unwrap();
    for (declaration, expected) in [
        (r#"{"template":"minecraft:piston"}"#, Some(3)),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"north"}}"#,
            Some(4),
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"north","extended":"true"}}"#,
            Some(5),
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"sideways"}}"#,
            None,
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"direction":"north"}}"#,
            None,
        ),
    ] {
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("horizon-mod.toml", &manifest("testmod", "")),
                ("data/testmod/horizon/block/test.json", declaration),
            ],
        );
        let mods = [m];
        match (Plan::new(&base, &mods), expected) {
            (Ok(plan), Some(id)) => {
                let mut doc: J = serde_json::from_str(facts).unwrap();
                plan.json("facts.json", &mut doc).unwrap();
                assert_eq!(
                    doc.get("defaults").unwrap().get("testmod:test"),
                    Some(&J::Num(id.into()))
                );
            }
            (Err(e), None) => assert!(e.to_string().contains("testmod:test"), "{e}"),
            _ => panic!("unexpected result for {declaration}"),
        }
    }
    // A malformed registry with duplicate states must not choose one silently.
    let duplicate = facts.replace(
        r#"["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"true"}}]"#,
        r#"["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"false"}}]"#,
    );
    fs::write(base.join("facts.json"), duplicate).unwrap();
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("horizon-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/horizon/block/test.json",
                r#"{"template":"minecraft:piston","default_state":{"facing":"north"}}"#,
            ),
        ],
    );
    let error = match Plan::new(&base, &[m]) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("duplicate states accepted"),
    };
    assert!(error.contains("matches 2 states"), "{error}");
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn shapes_reject_unknown_or_missing_template_values() {
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-shape-invalid-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    for declaration in [
        r#"{"template":"minecraft:oak_log","shapes":{"property":"typo","values":{"x":[]}}}"#,
        r#"{"template":"minecraft:oak_log","shapes":{"property":"axis","values":{"x":[]}}}"#,
        r#"{"template":"minecraft:oak_log","shapes":{"property":"axis","values":{"x":[],"y":[],"z":[],"bad":[]}}}"#,
    ] {
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("horizon-mod.toml", &manifest("shapetest", "")),
                ("data/shapetest/horizon/block/rod.json", declaration),
            ],
        );
        let mods = [m];
        let e = match Plan::new(&base, &mods) {
            Ok(_) => panic!("accepted {declaration}"),
            Err(e) => e.to_string(),
        };
        assert!(e.contains("shapetest:rod"), "{e}");
    }
    fs::remove_dir_all(scratch).unwrap();
}

/// A connecting block (template `minecraft:chorus_plant`, `connects_to` a
/// tag) joins itself and its tag's blocks on each side, needs no support, and
/// leaves vanilla's chorus plant as it was.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn connecting_blocks_join_themselves_and_their_tag() {
    use horizon_core::position::BlockPos;
    use horizon_core::random::LegacyRandom;
    use horizon_world::level::{Behaviours, LevelAccess, Ticks};
    use std::collections::HashMap;

    let Some(_) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-pipe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("cable");
    for (name, text) in [
        (
            "horizon-mod.toml",
            "id = \"cable\"\nname = \"Cable\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/cable/horizon/block/tin_cable.json",
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#cable:joins", "thickness": 4}"##,
        ),
        (
            "data/cable/horizon/block/machine.json",
            r#"{"template": "minecraft:iron_block"}"#,
        ),
        (
            "data/cable/tags/block/joins.json",
            r#"{"values": ["cable:machine"]}"#,
        ),
    ] {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mods::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "cable".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
    };
    let relative = Path::new("../../.local/horizon-client/26.3");
    let root = resource_root(relative, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let b = Behaviours::load(registry, &root).unwrap();

    struct Level {
        blocks: HashMap<BlockPos, BlockStateId>,
        air: BlockStateId,
        random: LegacyRandom,
    }
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.blocks.get(&pos).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            Some(self.blocks.insert(pos, state).unwrap_or(self.air))
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
    }
    let air = registry.default_state("minecraft:air").unwrap();
    let mut level = Level {
        blocks: HashMap::new(),
        air,
        random: LegacyRandom::new(1),
    };
    let mut ticks = Ticks::new(0);
    ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    let sides = |state: BlockStateId| {
        let s = registry.state(state).unwrap();
        ["down", "up", "north", "south", "west", "east"]
            .into_iter()
            .filter(|k| s.properties.get(*k).map(String::as_str) == Some("true"))
            .collect::<Vec<_>>()
    };
    let at = |x, y, z| BlockPos::new(x, y, z);
    let machine = registry.default_state("cable:machine").unwrap();
    let cable = registry.default_state("cable:tin_cable").unwrap();
    // A cable beside a machine (in its tag) and a stone joins the machine only,
    // and floats: no chorus plant support.
    b.set_block(&mut level, &mut ticks, at(1, 64, 0), machine, 3);
    b.set_block(
        &mut level,
        &mut ticks,
        at(-1, 64, 0),
        registry.default_state("minecraft:stone").unwrap(),
        3,
    );
    let placed = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(0, 64, 0), cable);
    b.set_block(&mut level, &mut ticks, at(0, 64, 0), placed, 3);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["east"]);
    assert!(b.can_survive(&mut level, &mut ticks, at(0, 64, 0), placed));
    // A second cable above: both join, through the neighbour's shape update.
    let above = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(0, 65, 0), cable);
    b.set_block(&mut level, &mut ticks, at(0, 65, 0), above, 3);
    assert_eq!(sides(level.block_state(at(0, 65, 0))), ["down"]);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["up", "east"]);
    // The machine gone: the arm goes too.
    b.set_block(&mut level, &mut ticks, at(1, 64, 0), air, 3);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["up"]);
    // Its shapes are 4 pixels across: the outline and the cached collision.
    let joined = level.block_state(at(0, 64, 0));
    let thin = horizon_world::collision::pipe_shape(4.0, [false, true, false, false, false, false])
        .unwrap();
    let facts = BlockFacts::load(&root, registry).unwrap();
    let outline = &facts.get(joined).outline;
    assert_eq!(outline.len(), thin.boxes().len());
    let file =
        horizon_world::registry::BlockTableFile::open(&root.join("cached-collision.nbt"), registry)
            .unwrap();
    let cached = horizon_world::collision::CachedCollisionRegistry::from_table(
        file.table("collision").unwrap(),
    )
    .unwrap();
    assert!(cached.has_canonical_identities());
    let shape = cached.cached_shape(joined).unwrap().unwrap();
    assert_eq!(shape.boxes(), thin.boxes());
    assert_eq!(
        shape
            .bounds()
            .map(|b| b.min(horizon_core::geometry::Axis::X)),
        Some(0.375)
    );
    // Vanilla's chorus plant keeps its own rules: it doesn't join the machine.
    b.set_block(&mut level, &mut ticks, at(5, 64, 0), machine, 3);
    let chorus = registry.default_state("minecraft:chorus_plant").unwrap();
    let chorus = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(4, 64, 0), chorus);
    assert!(sides(chorus).is_empty());
    fs::remove_dir_all(&scratch).unwrap();
}

/// An old chunk's saved jigsaw start comes back with its pieces (`StructureStart.loadStaticStart`):
/// a village this seed makes, saved as an older version would have, is loaded from its `Children`
/// and has the same pieces, boxes and positions, so the neighbours can place what crosses into
/// them. A legacy structure with no loader is the generator's own start when it has the same
/// pieces' box, and name and box alone when it has not.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn an_old_chunks_saved_village_comes_back_with_its_pieces() {
    use horizon_core::position::ChunkPos;
    use horizon_world::{
        nbt::Tag,
        worldgen::blending::{OldWorld, SavedStart},
    };
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(&base).unwrap();
    let open = || {
        crate::vanillagen::VanillaTerrain::new(
            &base.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
        )
        .unwrap()
    };
    let villages: Vec<String> = ["plains", "desert", "savanna", "snowy", "taiga"]
        .iter()
        .map(|k| format!("minecraft:village_{k}"))
        .collect();
    let made = open();
    let (name, at) = made
        .locate(&villages, [0, 64, 0], 100)
        .expect("a village near the origin");
    let (cx, cz) = (at[0] >> 4, at[2] >> 4);
    let original = made
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .expect("the village start");
    assert!(original.pieces.len() > 1);
    // Saved the way a chunk keeps it: the start's tag, its Children and their union box.
    let Some(Tag::Compound(saved)) = original.save() else {
        panic!("the village saves");
    };
    let Some(Tag::List(children)) = saved.get(&"Children".into()).cloned() else {
        panic!("Children");
    };
    let bounds = original
        .pieces
        .iter()
        .map(|p| p.bounding_box())
        .fold(None::<[i32; 6]>, |u, b| {
            let [x0, y0, z0, x1, y1, z1] =
                [b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]];
            Some(match u {
                None => [x0, y0, z0, x1, y1, z1],
                Some(u) => [
                    u[0].min(x0),
                    u[1].min(y0),
                    u[2].min(z0),
                    u[3].max(x1),
                    u[4].max(y1),
                    u[5].max(z1),
                ],
            })
        });
    struct Saved {
        chunk: (i32, i32),
        start: SavedStart,
    }
    impl OldWorld for Saved {
        fn is_old_chunk_around(&self, center: ChunkPos, range: i32) -> bool {
            (center.x - self.chunk.0).abs() <= range && (center.z - self.chunk.1).abs() <= range
        }
        fn blending(
            &self,
            _: i32,
            _: i32,
        ) -> Option<std::sync::Arc<horizon_world::worldgen::blending::BlendingData>> {
            None
        }
        fn old_biomes(
            &self,
            _: i32,
            _: i32,
        ) -> Option<
            std::sync::Arc<
                horizon_world::worldgen::biome_chunk::BiomeChunk<
                    horizon_world::worldgen::biome::BiomeId,
                >,
            >,
        > {
            None
        }
        fn saved_starts(&self, x: i32, z: i32) -> Vec<SavedStart> {
            if (x, z) == self.chunk {
                vec![self.start.clone()]
            } else {
                Vec::new()
            }
        }
    }
    let loaded = open();
    loaded.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        // No box to compare: the pieces can only come from the Children.
        start: SavedStart {
            name: name.clone(),
            bounds: None,
            pieces: children.clone(),
        },
    }));
    let back = loaded
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .expect("the saved start");
    assert_eq!(
        back.pieces.len(),
        original.pieces.len(),
        "every piece is loaded"
    );
    for (a, b) in original.pieces.iter().zip(&back.pieces) {
        assert_eq!(a.bounding_box(), b.bounding_box());
        assert_eq!(a.type_name(), b.type_name());
        assert_eq!(a.save(), b.save(), "a piece saves as it was loaded");
    }
    assert_eq!(back.bounding_box, original.bounding_box);
    // No pieces to load and a box no generated start has: the name and the box only.
    let odd = open();
    odd.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        start: SavedStart {
            name: name.clone(),
            bounds: Some([0, 0, 0, 1, 1, 1]),
            pieces: Vec::new(),
        },
    }));
    let stub = odd
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    assert!(stub.pieces.is_empty());
    assert_eq!(stub.bounding_box.min, [0, 0, 0]);
    // The box of the start this generator makes there: it is that start.
    let same = open();
    same.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        start: SavedStart {
            name: name.clone(),
            bounds,
            pieces: Vec::new(),
        },
    }));
    let rebuilt = same
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    assert_eq!(rebuilt.pieces.len(), original.pieces.len());
}

/// A recipe's `result` is a stack's fields or a bare item id, as vanilla's `ItemStackTemplate`
/// codec reads it (#4904): a mod's recipes make its own items either way.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn mod_recipe_results_are_an_item_stack_or_a_bare_item_id() {
    if prepared().is_none() {
        return;
    }
    let scratch = std::env::temp_dir().join(format!("horizon-mod-results-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("ruby");
    let _ = ruby_mod(&dir);
    // Each recipe takes one item no vanilla recipe crafts alone and makes one of the mod's.
    let cases = [
        (
            "stack",
            "minecraft:diamond",
            r#"{"id": "ruby:ruby"}"#,
            "ruby:ruby",
            1,
        ),
        (
            "bare_id",
            "minecraft:flint",
            r#""ruby:ruby""#,
            "ruby:ruby",
            1,
        ),
        (
            "bare_block_item",
            "minecraft:ghast_tear",
            r#""ruby:ruby_block""#,
            "ruby:ruby_block",
            1,
        ),
        (
            "counted",
            "minecraft:ender_pearl",
            r#"{"id": "ruby:ruby_block", "count": 3}"#,
            "ruby:ruby_block",
            3,
        ),
    ];
    for (name, input, result, _, _) in cases {
        let recipe = format!(
            r#"{{"type": "minecraft:crafting_shapeless", "ingredients": ["{input}"], "result": {result}}}"#
        );
        fs::write(dir.join(format!("data/ruby/recipe/{name}.json")), recipe).unwrap();
    }
    let package = horizon_mods::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "ruby".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
    };
    let relative = Path::new("../../.local/horizon-client/26.3");
    let root = resource_root(relative, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let new = assets.registry;
    let facts = BlockFacts::load(&root, new).unwrap();
    let items = Items::load(&root, new, &facts, &PackOverlay::NONE).unwrap();
    let recipes = crate::crafting::Recipes::load(&root, &items, &PackOverlay::NONE).unwrap();
    for (name, input, _, item, count) in cases {
        let grid = [
            Some(crate::items::ItemStack::new(input, 1)),
            None,
            None,
            None,
        ];
        let (_, made) = recipes
            .find(&grid, 2)
            .unwrap_or_else(|| panic!("{name}: no recipe"));
        assert_eq!((&*made.item, made.count), (item, count), "{name}");
    }
    fs::remove_dir_all(&scratch).unwrap();
}

/// A mod's creative tab comes after vanilla's, on page 1 in the first top
/// slot, with only the items the game has; its new items join the search tab
/// under their English names.
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn creative_tabs_come_on_their_own_page() {
    let Some(_) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-tabs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("gems");
    for (name, text) in [
        (
            "horizon-mod.toml",
            "id = \"gems\"\nname = \"Gems\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/gems/horizon/item/ruby.json",
            r#"{"template": "minecraft:diamond"}"#,
        ),
        (
            "data/gems/horizon/block/ruby_block.json",
            r#"{"template": "minecraft:diamond_block"}"#,
        ),
        (
            "data/gems/horizon/creative_tab/gems.json",
            r#"{"title": "Gems", "icon": "gems:ruby", "items": ["gems:ruby", "minecraft:diamond", "gems:missing"]}"#,
        ),
        (
            "assets/gems/lang/en_us.json",
            r#"{"item.gems.ruby": "Ruby"}"#,
        ),
    ] {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mods::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "gems".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
    };
    let relative = Path::new("../../.local/horizon-client/26.3");
    let root = resource_root(relative, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    let mut creative = crate::creative::Creative::load(&root).unwrap();
    assert_eq!(creative.pages(), 2);
    let i = creative
        .tabs
        .iter()
        .position(|t| t.id == "gems:gems")
        .unwrap();
    let tab = &creative.tabs[i];
    assert_eq!(
        (tab.page, tab.top, tab.column, tab.title.as_str()),
        (1, true, 0, "Gems")
    );
    let items: Vec<&str> = tab.items.iter().map(|s| &*s.item).collect();
    assert_eq!(items, ["gems:ruby", "minecraft:diamond"]);
    assert!(
        creative
            .search_items()
            .iter()
            .any(|s| &*s.item == "gems:ruby")
    );
    // A block's item is in search once.
    let blocks = creative
        .search_items()
        .iter()
        .filter(|s| &*s.item == "gems:ruby_block");
    assert_eq!(blocks.count(), 1);
    // Search, on every page, stays selected over a page turn.
    let search = creative.tabs.iter().position(|t| t.search).unwrap();
    creative.select(search);
    creative.set_page(1);
    assert_eq!((creative.selected, creative.page), (search, 1));
    creative.select(0);
    // Page 1 shows the mod's tab and the right-hand ones, not vanilla's.
    creative.set_page(1);
    assert_eq!(creative.selected, i);
    let shown: Vec<&str> = creative
        .tabs
        .iter()
        .filter(|t| creative.on_page(t))
        .map(|t| t.id.as_str())
        .collect();
    assert!(
        shown.contains(&"gems:gems") && shown.contains(&"minecraft:search"),
        "{shown:?}"
    );
    assert!(!shown.contains(&"minecraft:building_blocks"), "{shown:?}");
    // Selecting a vanilla tab goes back to its page.
    creative.select(0);
    assert_eq!(creative.page, 0);
    fs::remove_dir_all(&scratch).unwrap();
}

/// A block that gives each of its template's states a run moves a template
/// state to the first of its own: the combination every declared property is
/// at its default for, which is the one the id cursor points at first.
#[test]
fn a_fan_out_rename_moves_a_template_state_to_its_first_state() {
    let mut r = Rename::new("minecraft:test", "testmod:test");
    r.states = 10..12;
    r.first = 100;
    r.fanout = 6;
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(106), None)
    );
}

/// A base root of one template with two states, and a mod that adds a block
/// to it. `declarations` go in as the block file.
fn block_base(tag: &str, declarations: &[&str]) -> (PathBuf, Vec<ContentMod>) {
    block_base_of(
        tag,
        declarations,
        r#"{"states":[
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"x"}}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"y"}}]],
      "defaults":{"minecraft:test":0}}"#,
    )
}

/// The same over a base root whose `facts.json` is `facts`: the other tables
/// the plan reads are built from its state count, so a template can have one
/// state and no properties of its own (a `stone`) as easily as two.
fn block_base_of(tag: &str, declarations: &[&str], facts: &str) -> (PathBuf, Vec<ContentMod>) {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("facts.json"), facts).unwrap();
    let count = match serde_json::from_str::<J>(facts).unwrap().get("states") {
        Some(J::Arr(rows)) => rows.len(),
        _ => panic!("{tag}: facts.json has no states"),
    };
    let rows = vec![r#"{"item":"minecraft:test"}"#; count].join(",");
    fs::write(
        base.join("gameplay-facts.json"),
        format!(r#"{{"states":[{rows}]}}"#),
    )
    .unwrap();
    fs::write(base.join("items.json"), r#"{"minecraft:test":{}}"#).unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:item":{"entries":{"minecraft:test":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:test":{"protocol_id":0}}}}"#,
    )
    .unwrap();
    let mods: Vec<ContentMod> = declarations
        .iter()
        .enumerate()
        .map(|(i, declaration)| {
            let dir = scratch.join(format!("mod{i}"));
            folder_mod(
                &dir,
                &[
                    ("horizon-mod.toml", &manifest("testmod", "")),
                    ("data/testmod/horizon/block/test.json", *declaration),
                ],
            )
        })
        .collect();
    (base, mods)
}

/// A block's states are its template's times the combinations of the state
/// properties it declares, in a fixed order with every default first.
#[test]
fn content_block_state_properties_fan_out_its_states() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"},
        "facing":{"type":"enum","values":["north","south","east"]}}}"#;
    let (base, mods) = block_base("block-states", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    // Two template states times the six combinations, after the template's.
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    assert_eq!(rows.len(), 2 + 12);
    // The default is the first combination of the template's default state.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    let properties = |i: usize| -> BTreeMap<String, String> {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        let Some((_, J::Obj(p))) = state.iter().find(|(k, _)| k == "properties") else {
            panic!("row {i} has no properties")
        };
        p.iter()
            .map(|(k, v)| match v {
                J::Str(s) => (k.clone(), s.clone()),
                _ => panic!("{k} is not a string"),
            })
            .collect()
    };
    // By name `facing` varies slowest, and each one's default comes first.
    assert_eq!(
        properties(2),
        BTreeMap::from([
            ("axis".to_owned(), "x".to_owned()),
            ("facing".to_owned(), "north".to_owned()),
            ("lit".to_owned(), "false".to_owned()),
        ])
    );
    // And the last is the second template state and the last combination.
    assert_eq!(
        properties(13),
        BTreeMap::from([
            ("axis".to_owned(), "y".to_owned()),
            ("facing".to_owned(), "east".to_owned()),
            ("lit".to_owned(), "true".to_owned()),
        ])
    );
    // Every state of the block is a different one: the registry indexes
    // states by their name and properties, so a repeat would be a collision.
    let states: BTreeSet<BTreeMap<String, String>> = (2..14).map(properties).collect();
    assert_eq!(states.len(), 12);
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// The declared default comes first in the run, so its id is the block's
/// default with no arithmetic of its own.
#[test]
fn content_block_state_properties_put_the_declared_default_first() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"facing":{"type":"enum","values":["north","south","east"]}},
      "default_state":{"facing":"south"}}"#;
    let (base, mods) = block_base("block-default-first", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let facts = fs::read_to_string(base.join("facts.json")).unwrap();
    let mut doc: J = serde_json::from_str(&facts).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    // Two template states times three values.
    assert_eq!(rows.len(), 2 + 6);
    let facing = |i: usize| -> String {
        let J::Arr(row) = &rows[i] else {
            panic!("row {i} is not a state")
        };
        let Some(J::Obj(state)) = row.get(4) else {
            panic!("row {i} has no state")
        };
        match state
            .iter()
            .find(|(k, _)| k == "properties")
            .map(|(_, p)| p)
        {
            Some(p) => match p.get("facing") {
                Some(J::Str(s)) => s.clone(),
                _ => panic!("row {i} has no facing"),
            },
            None => panic!("row {i} has no properties"),
        }
    };
    // The declared default is the block's default, and it comes first.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(2.into()))
    );
    assert_eq!(facing(2), "south");
    // The rest keep their declared order, rotated to start at the default.
    assert_eq!(
        (facing(3), facing(4)),
        ("east".to_owned(), "north".to_owned())
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// What a declaration may not do: shadow its template's property, name a
/// value its own property does not have, or ask for more states than a world
/// holds.
#[test]
fn content_block_state_properties_are_validated() {
    // The template's `axis` is not the mod's to redefine.
    let (base, mods) = block_base(
        "block-shadow",
        &[r#"{"template":"minecraft:test","state_properties":{"axis":{"type":"boolean"}}}"#],
    );
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a shadowed template property was accepted"),
    };
    assert!(error.contains("axis is already on"), "{error}");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // A default is one of the values the block declared.
    let (base, mods) = block_base(
        "block-bad-default",
        &[r#"{"template":"minecraft:test",
             "state_properties":{"lit":{"type":"boolean"}},"default_state":{"lit":"maybe"}}"#],
    );
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a default outside a property's values was accepted"),
    };
    assert!(error.contains("is not one of its values"), "{error}");
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();

    // Twenty-one booleans are more combinations than a world can hold, and
    // are refused before the product is built.
    let declared: Vec<String> = (0..21)
        .map(|i| format!(r#""p{i}":{{"type":"boolean"}}"#))
        .collect();
    let declaration = format!(
        r#"{{"template":"minecraft:test","item":false,"state_properties":{{{}}}}}"#,
        declared.join(",")
    );
    let (base, mods) = block_base("block-too-many", &[&declaration]);
    let error = match Plan::new(&base, &mods) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("more states than a world holds were accepted"),
    };
    assert!(
        error.contains("more than the") && error.contains("world can hold"),
        "{error}"
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A `default_state` that names only properties the block itself declares
/// asks the template for nothing: `stone` has a single state and no properties
/// at all, and a block that gives it `lit` can still default `lit`.
#[test]
fn a_declared_property_defaults_on_a_template_with_none() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "default_state":{"lit":"true"}}"#;
    let (base, mods) = block_base_of(
        "block-propertyless-default",
        &[declaration],
        r#"{"states":[["minecraft:test",true,true,"",{"id":"minecraft:test"}]],
      "defaults":{"minecraft:test":0}}"#,
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let mut doc: J =
        serde_json::from_str(&fs::read_to_string(base.join("facts.json")).unwrap()).unwrap();
    plan.json("facts.json", &mut doc).unwrap();
    let Some(J::Arr(rows)) = doc.get("states") else {
        panic!("no states")
    };
    // One template state times the two combinations of `lit`.
    assert_eq!(rows.len(), 1 + 2);
    let lit = |i: usize| -> Option<String> {
        let J::Arr(row) = rows.get(i)? else {
            return None;
        };
        let J::Obj(state) = row.get(4)? else {
            return None;
        };
        let (_, properties) = state.iter().find(|(k, _)| k == "properties")?;
        match properties.get("lit") {
            Some(J::Str(s)) => Some(s.clone()),
            _ => None,
        }
    };
    // The declared default opens the block's run, and the other value follows.
    assert_eq!(lit(1).as_deref(), Some("true"));
    assert_eq!(lit(2).as_deref(), Some("false"));
    // The block's default state is the first of its run.
    assert_eq!(
        doc.get("defaults").unwrap().get("testmod:test"),
        Some(&J::Num(1.into()))
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

#[test]
fn named_definitions_follow_the_block_state_fanout() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "default_state":{"lit":"true"}}"#;
    let (base, mods) = block_base("named-state-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut models: J = serde_json::from_str(
        r#"{"minecraft:test":{
        "schema":{"axis":{"named":["x","y"]}},
        "states":[{"axis":"x"},{"axis":"y"}]}}"#,
    )
    .unwrap();
    plan.json("model-states.json", &mut models).unwrap();
    let added = models.get("testmod:test").unwrap();
    let expected: J = serde_json::from_str(
        r#"{"axis":{"named":["x","y"]},
        "lit":{"named":["false","true"]}}"#,
    )
    .unwrap();
    assert_eq!(added.get("schema"), Some(&expected));
    let expected: J = serde_json::from_str(
        r#"[
        {"axis":"x","lit":"true"},{"axis":"x","lit":"false"},
        {"axis":"y","lit":"true"},{"axis":"y","lit":"false"}]"#,
    )
    .unwrap();
    assert_eq!(added.get("states"), Some(&expected));

    let mut render: J = serde_json::from_str(
        r#"{"minecraft:test":{
        "default":0,"can_occlude":true,"bars_tag":false,"rule":"never",
        "states":[0,1]}}"#,
    )
    .unwrap();
    plan.json("render-definitions.json", &mut render).unwrap();
    let added = render.get("testmod:test").unwrap();
    let expected: J = serde_json::from_str("[2,3,4,5]").unwrap();
    assert_eq!(added.get("states"), Some(&expected));
    assert_eq!(added.get("default"), Some(&J::Num(2.into())));

    for (file, json) in [
        (
            "landing-definitions.json",
            r#"{"minecraft:test":{"fall":"Block","bounce":false}}"#,
        ),
        (
            "worldgen/blocks/survival-rules.json",
            r#"{"minecraft:test":"Always"}"#,
        ),
    ] {
        let mut doc: J = serde_json::from_str(json).unwrap();
        plan.json(file, &mut doc).unwrap();
        assert_eq!(doc.get("testmod:test"), doc.get("minecraft:test"), "{file}");
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// An NBT state row for the two-state `minecraft:test` template, the shape
/// `BlockState::to_nbt` writes.
fn test_state_row(axis: &str) -> Tag {
    Tag::Compound(Compound::from([
        (
            JavaString::from("id"),
            Tag::String(JavaString::from("minecraft:test")),
        ),
        (
            JavaString::from("properties"),
            Tag::Compound(Compound::from([(
                JavaString::from("axis"),
                Tag::String(JavaString::from(axis)),
            )])),
        ),
    ]))
}

/// The NBT tables fan out like the JSON ones: `cached-collision.nbt`,
/// `occlusion.nbt` and the rest get one row per combination, so `states` and
/// `cached_shapes` stay the same length, and each row carries the properties
/// its combination declares.
#[test]
fn the_nbt_tables_fan_out_with_the_states() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}}}"#;
    let (base, mods) = block_base("block-nbt-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("cached_shapes"), Tag::IntArray(vec![7, 9])),
    ]));
    plan.nbt(&mut tag);
    let Some(Tag::List(states)) = tag.get("states") else {
        panic!("no states")
    };
    let Some(Tag::IntArray(shapes)) = tag.get("cached_shapes") else {
        panic!("no cached_shapes")
    };
    // Two template states times the two combinations of `lit`, and the shapes
    // stay parallel to the states — that is what makes the file load.
    assert_eq!(states.len(), 2 + 4);
    assert_eq!(shapes.len(), states.len());
    // A combination keeps the shape of the template state it came from.
    assert_eq!(shapes, &vec![7, 9, 7, 7, 9, 9]);
    let properties = |i: usize| -> BTreeMap<String, String> {
        let Some(Tag::Compound(c)) = states.get(i) else {
            panic!("row {i} is not a compound")
        };
        let Some(Tag::Compound(p)) = c.get(&JavaString::from("properties")) else {
            panic!("row {i} has no properties")
        };
        p.iter()
            .filter_map(|(k, v)| match v {
                Tag::String(s) => Some((k.to_string_lossy(), s.to_string_lossy())),
                _ => None,
            })
            .collect()
    };
    let mut expected: Vec<String> = Vec::new();
    for axis in ["x", "y"] {
        for lit in ["false", "true"] {
            expected.push(format!("{axis}/{lit}"));
        }
    }
    let seen: Vec<String> = (2..states.len())
        .map(|i| {
            let p = properties(i);
            // The declared properties are the only ones on the row.
            assert_eq!(p.len(), 2, "row {i} is {p:?}");
            let axis = p.get("axis").cloned().unwrap_or_default();
            let lit = p.get("lit").cloned().unwrap_or_default();
            format!("{axis}/{lit}")
        })
        .collect();
    assert_eq!(seen, expected);
    for i in 2..states.len() {
        let Some(Tag::String(id)) = states[i].get("id") else {
            panic!("row {i} has no id")
        };
        assert_eq!(id.to_string_lossy(), "testmod:test");
    }
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A block's shape and its count in `structure-blocks.json` cover every
/// combination too: the table's declared total and the lengths of its lists
/// grow by the fan-out, and every state of the block maps to a shape.
#[test]
fn a_shape_and_the_state_count_cover_every_combination() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"lit":{"type":"boolean"}},
      "shapes":{"property":"axis","values":{"x":[[0,0,0,1,1,1]],"y":[[0,0,0,0.5,1,1]]}}}"#;
    let (base, mods) = block_base("block-shape-fanout", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    // The template's two states, no shapes yet, every vanilla state unmapped.
    // `nbt` grows the arrays to the block's four states and `collision_shapes`
    // writes into them, in the order the real path applies them.
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("shapes"), Tag::List(Vec::new())),
        (
            JavaString::from("cached_shapes"),
            Tag::IntArray(vec![-1, -1]),
        ),
    ]));
    plan.nbt(&mut tag);
    plan.collision_shapes(&mut tag).unwrap();
    let Some(Tag::List(entries)) = tag.get("shapes") else {
        panic!("no shapes")
    };
    let Some(Tag::IntArray(mapping)) = tag.get("cached_shapes") else {
        panic!("no cached_shapes")
    };
    // One shape per template state, shared by that state's combinations.
    assert_eq!(entries.len(), 2);
    assert_eq!(mapping.len(), 2 + 4);
    let added = &mapping[2..];
    assert!(added.iter().all(|id| *id >= 0), "{added:?}");
    assert_eq!((added[0], added[1]), (added[0], added[0]));
    assert_eq!((added[2], added[3]), (added[2], added[2]));
    assert_ne!(added[0], added[2]);

    let rows = "[[false,false],[false,false]]";
    let text = format!(r#"{{"rotate":{rows},"mirror":{rows},"full_block":{rows},"states":2}}"#);
    let mut doc: J = serde_json::from_str(&text).unwrap();
    plan.json("worldgen/blocks/structure-blocks.json", &mut doc)
        .unwrap();
    for key in ["rotate", "mirror", "full_block"] {
        let Some(J::Arr(list)) = doc.get(key) else {
            panic!("{key} is not a list")
        };
        assert_eq!(list.len(), 2 + 4, "{key}");
    }
    // The declared total agrees with the lists, which is what a loader checks.
    assert_eq!(doc.get("states"), Some(&J::Num(6.into())));
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(base.parent().unwrap().join("mod0")).ok();
}

/// A base root of one block with three biomes, two biome tags (one nesting
/// the other) and three vanilla placed features: what biome modifiers read.
fn biome_base(tag: &str) -> PathBuf {
    let (base, _) = block_base(tag, &[]);
    let worldgen = base.join("worldgen");
    fs::create_dir_all(&worldgen).unwrap();
    let bundle = |entries: &[(&str, &str)]| -> String {
        let map: BTreeMap<&str, &str> = entries.iter().copied().collect();
        serde_json::to_string(&map).unwrap()
    };
    let files = [
        (
            "biomes.json",
            bundle(&[
                (
                    "minecraft:plains",
                    r#"{"temperature":0.8,"features":[[],["minecraft:lake"],[],[],[],[],["minecraft:ore_coal"]]}"#,
                ),
                ("minecraft:desert", r#"{"features":[["minecraft:fossil"]]}"#),
                (
                    "minecraft:nether_wastes",
                    r#"{"temperature":2.0,"features":[[],[],[],[],[],[],["minecraft:ore_coal"]]}"#,
                ),
            ]),
        ),
        (
            "biome_tags.json",
            bundle(&[
                (
                    "minecraft:is_overworld",
                    r##"{"values":["#minecraft:is_plains_like","minecraft:desert",{"id":"othermod:x","required":false}]}"##,
                ),
                (
                    "minecraft:is_plains_like",
                    r#"{"values":["minecraft:plains"]}"#,
                ),
            ]),
        ),
        (
            "placed_features.json",
            bundle(&[
                ("minecraft:ore_coal", "{}"),
                ("minecraft:lake", "{}"),
                ("minecraft:fossil", "{}"),
            ]),
        ),
        ("features.json", bundle(&[])),
    ];
    for (name, text) in files {
        fs::write(worldgen.join(name), text).unwrap();
    }
    base
}

/// A mod adding a zinc ore and its biome modifiers (`(file name, text)`).
fn zinc_mod(dir: &Path, modifiers: &[(&str, &str)]) -> ContentMod {
    let mut files = vec![
        ("horizon-mod.toml".to_owned(), manifest("zinc", "")),
        (
            "data/zinc/worldgen/configured_feature/ore_zinc.json".to_owned(),
            r#"{"type":"minecraft:ore","config":{}}"#.to_owned(),
        ),
        (
            "data/zinc/worldgen/placed_feature/ore_zinc.json".to_owned(),
            r#"{"feature":"zinc:ore_zinc","placement":[]}"#.to_owned(),
        ),
    ];
    for (name, text) in modifiers {
        files.push((
            format!("data/zinc/neoforge/biome_modifier/{name}.json"),
            (*text).to_owned(),
        ));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    folder_mod(dir, &files)
}

fn biome_features(root: &Path) -> BTreeMap<String, serde_json::Value> {
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap();
    bundle
        .into_iter()
        .map(|(id, text)| {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            (id, v["features"].clone())
        })
        .collect()
}

/// `neoforge:add_features` puts a mod's ore at the end of its step in every
/// biome of an overworld tag, `remove_features` takes a vanilla feature out,
/// and the biomes no modifier names keep their text.
#[test]
fn biome_modifiers_place_mod_features_in_vanilla_biomes() {
    let base = biome_base("biome-modifiers");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            (
                "add_zinc",
                r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_overworld",
                    "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            ),
            // Adds before removes, whatever the file order.
            (
                "a_no_fossils",
                r#"{"type":"neoforge:remove_features","biomes":["minecraft:desert"],
                    "features":["minecraft:fossil"],"steps":["raw_generation"]}"#,
            ),
            ("nothing", r#"{"type":"neoforge:none"}"#),
        ],
    );
    let out = scratch.join("out");
    build(&base, &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"],
        serde_json::json!([
            [],
            ["minecraft:lake"],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    // A biome with fewer steps gets the ones up to the modifier's.
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );
    let text = |root: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap()
    };
    assert_eq!(
        text(&out)["minecraft:nether_wastes"],
        text(&base)["minecraft:nether_wastes"]
    );
    let plains: serde_json::Value = serde_json::from_str(&text(&out)["minecraft:plains"]).unwrap();
    assert_eq!(plains["temperature"], serde_json::json!(0.8));

    // The generator's sorter places vanilla's features where they were and
    // the ore after the coal it follows: the order applyBiomeDecoration uses.
    use horizon_world::worldgen::feature_sort::{FeatureIdentity, build_features_per_step};
    let mut ids: Vec<String> = Vec::new();
    let mut plan = |biomes: &[&str], features: &BTreeMap<String, serde_json::Value>| {
        let lists: Vec<Vec<Vec<FeatureIdentity>>> = biomes
            .iter()
            .map(|b| {
                features[*b]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|step| {
                        step.as_array()
                            .unwrap()
                            .iter()
                            .map(|f| {
                                let f = f.as_str().unwrap().to_owned();
                                let i = ids.iter().position(|x| *x == f).unwrap_or_else(|| {
                                    ids.push(f);
                                    ids.len() - 1
                                });
                                FeatureIdentity(i as u32)
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        build_features_per_step(&lists).unwrap()
    };
    let overworld = ["minecraft:plains", "minecraft:desert"];
    let steps = plan(&overworld, &features);
    let names = |s: usize, ids: &[String]| -> Vec<String> {
        steps[s]
            .features()
            .iter()
            .map(|f| ids[f.0 as usize].clone())
            .collect()
    };
    assert_eq!(names(6, &ids), ["minecraft:ore_coal", "zinc:ore_zinc"]);
    assert_eq!(names(1, &ids), ["minecraft:lake"]);
    let _ = fs::remove_dir_all(&scratch);
}

/// A mod with features but no modifier leaves vanilla's biome bundle alone.
#[test]
fn biome_modifiers_none_keep_the_vanilla_biomes() {
    let base = biome_base("biome-modifiers-none");
    let scratch = base.parent().unwrap().to_owned();
    let out = scratch.join("out");
    build(&base, &[zinc_mod(&scratch.join("zinc"), &[])], &out).unwrap();
    assert_eq!(
        fs::read(out.join("worldgen/biomes.json")).unwrap(),
        fs::read(base.join("worldgen/biomes.json")).unwrap()
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// An unknown biome, biome tag, feature, step or modifier type fails the
/// build with the mod and file named.
#[test]
fn biome_modifiers_refuse_unknown_names() {
    let base = biome_base("biome-modifiers-errors");
    let scratch = base.parent().unwrap().to_owned();
    // A name the root doesn't have, or a type not read yet: that modifier is
    // skipped with a warning (another mod's ids, `#c:` tags and `add_spawns`
    // are common in real mods), and the mods still load.
    for (i, (modifier, warning)) in [
        (
            r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_moon",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            "unknown biome tag #minecraft:is_moon",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:moon",
                "features":"zinc:ore_zinc","step":"underground_ores"}"#,
            "unknown biome minecraft:moon",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":"zinc:ore_tin","step":"underground_ores"}"#,
            "unknown placed feature zinc:ore_tin",
        ),
        (
            r#"{"type":"neoforge:add_spawns","biomes":"minecraft:plains","spawners":[]}"#,
            "biome modifier type \"add_spawns\" skipped",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let m = zinc_mod(&scratch.join(format!("zinc{i}")), &[("bad", modifier)]);
        let warnings = build(&base, &[m], &scratch.join(format!("out{i}"))).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains(warning)
                && w.contains("mod zinc: data/zinc/neoforge/biome_modifier/bad.json")),
            "{warnings:?}"
        );
    }
    // A malformed modifier of a type Horizon reads is the mod's error.
    let m = zinc_mod(
        &scratch.join("zinc-step"),
        &[(
            "bad",
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":"zinc:ore_zinc","step":"ores"}"#,
        )],
    );
    let e = build(&base, &[m], &scratch.join("out-step"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown generation step \"ores\""), "{e}");
    assert!(
        e.contains("mod zinc: data/zinc/neoforge/biome_modifier/bad.json"),
        "{e}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Over the real prepared resources: a mod's ore, added to
/// `#minecraft:is_overworld` by a biome modifier, generates in a fresh
/// world's chunk; without the modifier the same placed feature does not
/// (its `biome` filter only passes in biomes that list it).
#[test]
#[ignore = "full content root: CI and pr_ready run it"]
fn content_root_generates_a_biome_modifier_ore() {
    use crate::streaming::{ChunkGenerator, Proto};
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-ore-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let ore = |modifier: bool| -> usize {
        let mut files = vec![
            ("horizon-mod.toml", manifest("zinc", "")),
            (
                "data/zinc/horizon/block/zinc_ore.json",
                r#"{"template": "minecraft:iron_ore"}"#.to_owned(),
            ),
            (
                "data/zinc/worldgen/configured_feature/ore_zinc.json",
                r##"{"type": "minecraft:ore", "size": 9, "discard_chance_on_air_exposure": 0.0,
                    "targets": [{"target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:stone_ore_replaceables"},
                        "state": "zinc:zinc_ore"}]}"##
                    .to_owned(),
            ),
            (
                "data/zinc/worldgen/placed_feature/ore_zinc.json",
                r#"{"feature": "zinc:ore_zinc", "placement": [
                    {"type": "minecraft:count", "count": 40}, {"type": "minecraft:in_square"},
                    {"type": "minecraft:height_range", "height": {"type": "minecraft:uniform",
                        "min_inclusive": {"absolute": -32}, "max_inclusive": {"absolute": 48}}},
                    {"type": "minecraft:biome"}]}"#
                    .to_owned(),
            ),
        ];
        if modifier {
            files.push((
                "data/zinc/neoforge/biome_modifier/add_zinc.json",
                r##"{"type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
                    "features": "zinc:ore_zinc", "step": "underground_ores"}"##
                    .to_owned(),
            ));
        }
        let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (*a, b.as_str())).collect();
        let dir = scratch.join(format!("zinc-{modifier}"));
        let m = folder_mod(&dir, &files);
        let root = resource_root(&base, &[m], &dir.join("cache")).unwrap();
        let assets = Assets::load(&root).unwrap();
        let zinc = assets.registry.default_state("zinc:zinc_ore").unwrap().0;
        let biomes = crate::biome::Biomes::load(&root).unwrap();
        let terrain = crate::vanillagen::VanillaTerrain::new(
            &root.join("worldgen"),
            assets.registry,
            &biomes,
            1234,
        )
        .unwrap();
        let mut worker = terrain.worker();
        let mut region: [Proto; 9] =
            std::array::from_fn(|i| terrain.proto(&mut worker, i as i32 % 3 - 1, i as i32 / 3 - 1));
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut count = 0;
        for y in chunk.min_y()..chunk.max_y() {
            for z in 0..16 {
                for x in 0..16 {
                    count += usize::from(chunk.get(x, y, z) == zinc);
                }
            }
        }
        count
    };
    assert!(ore(true) > 0, "the modifier's ore generates");
    assert_eq!(ore(false), 0, "no modifier, no ore");
    let _ = fs::remove_dir_all(&scratch);
}

/// `biomes` may be one id or a list of ids as well as a tag (NeoForge's
/// HolderSet forms); an id may leave out `minecraft:`.
#[test]
fn biome_modifiers_take_one_biome_or_a_list() {
    let base = biome_base("biome-modifiers-lists");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            (
                "list",
                r#"{"type":"neoforge:add_features","biomes":["minecraft:plains","nether_wastes"],
                    "features":"zinc:ore_zinc","step":"underground_ores"}"#,
            ),
            (
                "one",
                r#"{"type":"neoforge:add_features","biomes":"minecraft:desert",
                    "features":["minecraft:lake"],"step":"lakes"}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    build(&base, &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([["minecraft:fossil"], ["minecraft:lake"]])
    );
    let _ = fs::remove_dir_all(&scratch);
}
