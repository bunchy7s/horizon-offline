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
        assert_eq!(e, "minecraft:fod is no component a vanilla item has");
    }
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
            "minecraft:iron_block":{"protocol_id":11}}}}"#,
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
                "minecraft:iron_block":{"protocol_id":11}}}}"#,
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
/// templates, an item, a vanilla tag addition and a language file.
fn ruby_mod(dir: &Path) -> ContentMod {
    let files: [(&str, &str); 9] = [
        (
            "horizon-mod.toml",
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/ruby/horizon/block/ruby_block.json",
            r#"{"template": "minecraft:iron_block", "properties": {"hardness": 4.0, "light": 3,
                "sounds": "minecraft:amethyst_block", "map_color": 28, "friction": 0.8}}"#,
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
        BlockFacts::load_with_projection(&root, new, &shared_json).unwrap();
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
    let gem = &items.items["ruby:ruby"];
    assert_eq!(gem.max_stack, 16);
    let sword = &items.items["ruby:ruby_sword"];
    let iron_sword = &items.items["minecraft:iron_sword"];
    assert_eq!((sword.max_damage, iron_sword.max_damage), (900, 250));
    assert_eq!(sword.modifiers.len(), 1);
    assert_eq!(sword.modifiers[0].2, 7.0);
    assert_eq!(sword.tool.len(), iron_sword.tool.len());
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
        BlockFacts::load_with_projection(&root, assets.registry, &doc).unwrap();
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
        // Additions to vanilla's set and pool; a target nobody defines is skipped.
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
            serde_json::json!({"structure": "huts:hut", "weight": 3}),
        ),
        (
            "template_pools.json",
            "minecraft:village/plains/houses",
            "elements",
            serde_json::json!({"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}),
        ),
    ] {
        let (ours, vanilla) = (bundle(name, &root), bundle(name, &base));
        let entries = |text: &str| -> Vec<serde_json::Value> {
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            v[list].as_array().unwrap().clone()
        };
        let mut expected = entries(&vanilla[id]);
        expected.push(added);
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
