use super::{args::*, vanilla, *};

fn reg() -> Registries {
    let mut r = Registries {
        permission: 4,
        ..Registries::default()
    };
    r.items = vec![
        "minecraft:diamond".into(),
        "minecraft:diamond_sword".into(),
        "minecraft:stone".into(),
    ];
    r.blocks.insert("minecraft:stone".into(), vec![]);
    r.blocks.insert(
        "minecraft:oak_stairs".into(),
        vec![
            ("facing".into(), vec!["north".into(), "south".into()]),
            ("half".into(), vec!["top".into(), "bottom".into()]),
        ],
    );
    r.summonable = vec!["minecraft:pig".into(), "minecraft:zombie".into()];
    r.entity_types = vec![
        "minecraft:pig".into(),
        "minecraft:player".into(),
        "minecraft:zombie".into(),
    ];
    r.players = vec!["Player".into()];
    r.dimensions = vec!["minecraft:overworld".into()];
    r.time_markers = vec!["minecraft:day".into(), "minecraft:noon".into()];
    // The registry elements the commands below name (vanilla checks them while parsing).
    let mut registry = |name: &str, ids: &[&str]| {
        r.by_registry.insert(
            name.into(),
            RegistryIds {
                ids: ids.iter().map(|s| (*s).to_owned()).collect(),
                tags: Vec::new(),
            },
        );
    };
    registry(
        "minecraft:worldgen/biome",
        &["minecraft:desert", "minecraft:plains"],
    );
    registry(
        "minecraft:damage_type",
        &["minecraft:fall", "minecraft:magic"],
    );
    registry(
        "minecraft:attribute",
        &["minecraft:max_health", "minecraft:scale"],
    );
    registry("minecraft:item_modifier", &["a:b"]);
    registry("minecraft:predicate", &["a:b"]);
    registry(
        "minecraft:loot_table",
        &[
            "minecraft:chests/simple_dungeon",
            "minecraft:chests/x",
            "minecraft:gameplay/fishing",
        ],
    );
    registry(
        "minecraft:entity_type",
        &["minecraft:pig", "minecraft:player", "minecraft:zombie"],
    );
    registry(
        "minecraft:item",
        &[
            "minecraft:diamond",
            "minecraft:diamond_sword",
            "minecraft:stone",
        ],
    );
    r
}
fn run(input: &str) -> CmdResult<(String, Vec<String>)> {
    let reg = reg();
    let parse = vanilla::dispatcher().parse(input, &reg);
    let (ctx, id) = parse.executable()?;
    Ok((
        id.to_owned(),
        ctx.args.iter().map(|a| a.name.clone()).collect(),
    ))
}
fn suggestions(input: &str) -> Vec<String> {
    let reg = reg();
    let d = vanilla::dispatcher();
    let parse = d.parse(input, &reg);
    d.suggestions(&parse, input.len(), &reg)
        .1
        .into_iter()
        .map(|s| s.text)
        .collect()
}

#[test]
fn reader_matches_brigadier() {
    let mut r = Reader::new("12 -3.5 abc \"q\\\"x\" true");
    assert_eq!(r.read_int().unwrap(), 12);
    r.skip();
    assert_eq!(r.read_double().unwrap(), -3.5);
    r.skip();
    assert_eq!(r.read_unquoted_string(), "abc");
    r.skip();
    assert_eq!(r.read_string().unwrap(), "q\"x");
    r.skip();
    assert!(r.read_boolean().unwrap());
    let e = Reader::new("1.5").read_int().unwrap_err();
    assert_eq!(e.message.plain(), "parsing.int.invalid");
    assert_eq!(e.context().unwrap(), "<--[HERE]");
}
#[test]
fn parses_vanilla_commands() {
    assert_eq!(run("gamemode creative").unwrap().0, "gamemode");
    assert_eq!(
        run("gamemode creative @s").unwrap().1,
        ["gamemode", "target"]
    );
    assert_eq!(
        run("give @s minecraft:diamond 64").unwrap().1,
        ["targets", "item", "count"]
    );
    assert_eq!(run("tp 1 2 3").unwrap().0, "teleport.location");
    assert_eq!(run("tp Player").unwrap().0, "teleport.destination");
    assert_eq!(run("tp @s ~ ~10 ~").unwrap().0, "teleport.targets.location");
    assert_eq!(
        run("teleport @e[type=pig,limit=2] ^ ^ ^5 90 0").unwrap().0,
        "teleport.targets.location"
    );
    assert_eq!(run("time set 1000").unwrap().0, "time.set");
    assert_eq!(run("time set 1d").unwrap().0, "time.set");
    assert_eq!(run("time set day").unwrap().0, "time.set.marker");
    assert_eq!(run("time query gametime").unwrap().0, "time.query.gametime");
    assert_eq!(run("weather rain 10s").unwrap().0, "weather");
    assert_eq!(run("xp add @s 5 levels").unwrap().0, "experience");
    assert_eq!(run("gamerule keep_inventory true").unwrap().0, "gamerule");
    assert_eq!(
        run("gamerule minecraft:random_tick_speed 10").unwrap().0,
        "gamerule"
    );
    assert_eq!(run("fill ~ ~ ~ ~1 ~1 ~1 stone hollow").unwrap().0, "fill");
    assert_eq!(
        run("fill 0 0 0 1 1 1 minecraft:oak_stairs[facing=north] replace stone")
            .unwrap()
            .0,
        "fill"
    );
    assert_eq!(
        run("clone 0 0 0 1 1 1 5 5 5 masked move").unwrap().0,
        "clone"
    );
    assert_eq!(
        run("setblock ~ ~1 ~ oak_stairs[half=top] keep").unwrap().0,
        "setblock"
    );
    assert_eq!(run("summon zombie ~ ~ ~ {NoAI:1b}").unwrap().0, "summon");
    assert_eq!(run("difficulty hard").unwrap().0, "difficulty");
    assert_eq!(run("say hello @p").unwrap().0, "say");
    let reg = reg();
    let parse =
        vanilla::dispatcher().parse("clone 0 0 0 1 1 1 5 5 5 strict filtered stone force", &reg);
    let (ctx, _) = parse.executable().unwrap();
    assert!(ctx.has("strict") && ctx.has("filtered") && ctx.has("force") && !ctx.has("move"));
}
#[test]
fn vanilla_error_messages() {
    let e = run("foo").unwrap_err();
    assert_eq!(e.message.plain(), "command.unknown.command");
    assert_eq!(e.cursor, 0);
    let e = run("gamemode").unwrap_err();
    assert_eq!(e.message.plain(), "command.unknown.command");
    let e = run("gamemode creatve").unwrap_err();
    assert_eq!(e.message.plain(), "argument.gamemode.invalid");
    // GameModeArgument reports the error after the word.
    assert_eq!(e.cursor, 16);
    let e = run("give @s minecraft:dirt").unwrap_err();
    assert_eq!(e.message.plain(), "argument.item.id.invalid");
    let e = run("give @s diamond 0").unwrap_err();
    assert_eq!(e.message.plain(), "argument.integer.low");
    let e = run("gamemode creative @e").unwrap_err();
    assert_eq!(e.message.plain(), "argument.player.entities");
    let e = run("setblock 1 2").unwrap_err();
    assert_eq!(e.message.plain(), "argument.pos3d.incomplete");
    let e = run("setblock 1 2 3 oak_stairs[facing=up]").unwrap_err();
    assert_eq!(e.message.plain(), "argument.block.property.invalid");
}
#[test]
fn commands_need_allow_commands() {
    // IntegratedServer.getProfilePermissions: the owner of a world without
    // Allow Commands is at LevelBasedPermissionSet.ALL.
    let reg = Registries {
        permission: 0,
        ..reg()
    };
    let d = vanilla::game_dispatcher();
    let e = d.parse("gamemode creative", &reg).executable().unwrap_err();
    assert_eq!(e.message.plain(), "command.unknown.command");
    assert!(d.parse("me waves", &reg).executable().is_ok());
    let parse = d.parse("", &reg);
    // Brigadier itself lists every literal; the chat's (client tree) only
    // the ones the player may use.
    assert!(
        d.suggestions(&parse, 0, &reg)
            .1
            .iter()
            .any(|s| s.text == "gamemode")
    );
    let root: Vec<String> = d
        .client_suggestions(&parse, 0, &reg)
        .1
        .into_iter()
        .map(|s| s.text)
        .collect();
    // Commands.java with CommandSelection.INTEGRATED: seed and version need no
    // level (a dedicated server's tree: a_dedicated_server_keeps_seed_and_version_for_game_masters).
    assert_eq!(
        root,
        [
            "help", "list", "me", "msg", "random", "seed", "teammsg", "tell", "tm", "trigger",
            "version", "w"
        ]
    );
}
#[test]
fn suggestions_like_vanilla() {
    let root = suggestions("");
    assert!(root.contains(&"gamemode".to_owned()) && root.contains(&"tp".to_owned()));
    assert_eq!(suggestions("game"), ["gamemode", "gamerule"]);
    assert_eq!(
        suggestions("gamemode "),
        ["adventure", "creative", "spectator", "survival"]
    );
    assert_eq!(suggestions("gamemode c"), ["creative"]);
    assert_eq!(
        suggestions("give @s dia"),
        ["minecraft:diamond", "minecraft:diamond_sword"]
    );
    // Underscore-separated parts match too (SharedSuggestionProvider.matchesSubStr).
    assert_eq!(suggestions("give @s sword"), ["minecraft:diamond_sword"]);
    assert_eq!(suggestions("kill @"), ["@a", "@e", "@n", "@p", "@r", "@s"]);
    assert_eq!(suggestions("kill @e"), ["["]);
    assert_eq!(suggestions("kill @e[ty"), ["type="]);
    assert!(suggestions("kill @e[type=").contains(&"minecraft:zombie".to_owned()));
    assert_eq!(suggestions("kill @e[type=pig"), [",", "]"]);
    assert_eq!(suggestions("setblock 0 0 0 oak_stairs[fa"), ["facing="]);
    // BlockStateParser suggests every value of the property, unfiltered.
    assert_eq!(
        suggestions("setblock 0 0 0 oak_stairs[facing=n"),
        ["north", "south"]
    );
    assert_eq!(
        suggestions("tp "),
        [
            "@a", "@e", "@n", "@p", "@r", "@s", "Player", "~", "~ ~", "~ ~ ~"
        ]
    );
    assert_eq!(suggestions("time set 5"), ["d", "s", "t"]);
    assert!(suggestions("time set ").contains(&"minecraft:day".to_owned()));
}
/// ClientSuggestionProvider: block and column positions offer the targeted
/// block (`getRelevantCoordinates`), `Vec3` and `Vec2` the exact hit point
/// with two decimals (`getAbsoluteCoordinates`); with no block hit, `~`.
#[test]
fn coordinates_suggest_the_crosshair_block_or_hit_point() {
    let d = vanilla::dispatcher();
    let suggest = |reg: &Registries, input: &str| -> Vec<String> {
        let parse = d.parse(input, reg);
        d.suggestions(&parse, input.len(), reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect()
    };
    let mut hit = reg();
    hit.target_block = Some([-29, 73, 8]);
    hit.target_point = Some(["-28.77".into(), "73.28".into(), "8.00".into()]);
    assert_eq!(suggest(&hit, "setblock "), ["-29", "-29 73", "-29 73 8"]);
    assert_eq!(
        suggest(&hit, "summon minecraft:pig "),
        ["-28.77", "-28.77 73.28", "-28.77 73.28 8.00"]
    );
    assert_eq!(
        suggest(&hit, "summon minecraft:pig -28.77 "),
        ["-28.77 73.28", "-28.77 73.28 8.00"]
    );
    // Vec2Argument: x and z.
    assert_eq!(
        suggest(&hit, "worldborder center "),
        ["-28.77", "-28.77 8.00"]
    );
    // An entity in front of the block (or no hit): the defaults.
    let none = reg();
    assert_eq!(suggest(&none, "setblock "), ["~", "~ ~", "~ ~ ~"]);
    assert_eq!(
        suggest(&none, "summon minecraft:pig "),
        ["~", "~ ~", "~ ~ ~"]
    );
}
#[test]
fn entity_arguments_suggest_the_selected_entity() {
    // EntityArgument.listSuggestions: getSelectedEntities (the crosshair
    // entity's UUID) joins the player names unless the argument takes
    // players only; GameProfileArgument and ScoreHolderArgument never ask.
    let uuid = "0b9c6a7e-3d1f-4a2b-9c8d-7e6f5a4b3c2d";
    let mut reg = reg();
    reg.selected_entities = vec![uuid.into()];
    let d = vanilla::dispatcher();
    let suggest = |input: &str| -> Vec<String> {
        let parse = d.parse(input, &reg);
        d.suggestions(&parse, input.len(), &reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect()
    };
    assert_eq!(
        suggest("tp "),
        [
            "0b9c6a7e-3d1f-4a2b-9c8d-7e6f5a4b3c2d",
            "@a",
            "@e",
            "@n",
            "@p",
            "@r",
            "@s",
            "Player",
            "~",
            "~ ~",
            "~ ~ ~"
        ]
    );
    assert!(suggest("kill ").contains(&uuid.to_owned()));
    assert_eq!(suggest("kill 0b9"), [uuid]);
    for players_only in ["gamemode creative ", "op ", "scoreboard players set "] {
        assert!(
            !suggest(players_only).contains(&uuid.to_owned()),
            "{players_only}"
        );
    }
}
#[test]
fn a_uuid_selects_by_entity_uuid() {
    // EntitySelectorParser.parseNameOrUUID: a UUID sets entityUUID and
    // includesEntities, never playerName (which findEntities checks first).
    let reg = reg();
    let mut r = Reader::new("2de5a9ad-2740-4058-835e-cf255e0d7798");
    let s = selector::parse(&mut r, &reg, true).unwrap();
    assert_eq!(s.uuid, Some(0x2de5a9ad_2740_4058_835e_cf255e0d7798));
    assert!(s.include_entities && s.player_name.is_none());
    let mut r = Reader::new("Steve");
    let s = selector::parse(&mut r, &reg, true).unwrap();
    assert_eq!((s.uuid, s.player_name.as_deref()), (None, Some("Steve")));
}
#[test]
fn smart_usage_matches_brigadier() {
    let d = vanilla::dispatcher();
    let usage = |name: &str| {
        d.smart_usage(0, 4)
            .into_iter()
            .find(|(n, _)| d.nodes[*n].name() == name)
            .unwrap()
            .1
    };
    assert_eq!(usage("gamemode"), "gamemode <gamemode> [<target>]");
    assert_eq!(usage("give"), "give <targets> <item> [<count>]");
    assert_eq!(usage("kill"), "kill [<targets>]");
    assert_eq!(usage("weather"), "weather (clear|rain|thunder)");
    assert_eq!(usage("tp"), "tp -> teleport");
    assert_eq!(
        usage("difficulty"),
        "difficulty [peaceful|easy|normal|hard]"
    );
}
#[test]
fn local_coordinates_follow_view() {
    // Facing south (yaw 0) moves +Z.
    let p = Coordinates::Local([0.0, 0.0, 2.0]).position([0.0, 64.0, 0.0], [0.0, 0.0]);
    assert!((p[2] - 2.0).abs() < 1e-6 && p[0].abs() < 1e-6);
    // ^1 (left) while facing south is +X.
    let p = Coordinates::Local([1.0, 0.0, 0.0]).position([0.0, 64.0, 0.0], [0.0, 0.0]);
    assert!((p[0] - 1.0).abs() < 1e-6, "{p:?}");
    let w = Coordinates::World([
        Coord {
            relative: true,
            value: 1.0,
        },
        Coord {
            relative: false,
            value: 5.0,
        },
        Coord {
            relative: true,
            value: 0.0,
        },
    ]);
    assert_eq!(w.position([10.0, 64.0, 3.0], [0.0, 0.0]), [11.0, 5.0, 3.0]);
}

#[test]
fn data_and_tag_commands_parse() {
    let (id, args) = run("data get entity @s Pos 2.5").unwrap();
    assert_eq!(id, "data.get");
    assert_eq!(args, ["target", "path", "scale"]);
    assert_eq!(run("data get block ~ ~ ~ Items[0]").unwrap().0, "data.get");
    assert_eq!(
        run("item modify entity @s weapon.mainhand a:b").unwrap().0,
        "item.modify"
    );
    assert_eq!(
        run("item modify block 1 2 3 container.0 a:b").unwrap().0,
        "item.modify"
    );
    assert_eq!(run("data get storage my:store").unwrap().0, "data.get");
    assert_eq!(run("data merge storage a:b {x:1}").unwrap().0, "data.merge");
    assert_eq!(
        run("data remove entity @s foo.bar[1]").unwrap().0,
        "data.remove"
    );
    let (id, args) = run("data modify storage a:b x.y insert 2 from entity @s Pos[0]").unwrap();
    assert_eq!(id, "data.modify");
    assert_eq!(
        args,
        ["target", "targetPath", "index", "source", "sourcePath"]
    );
    assert_eq!(
        run("data modify block 1 2 3 Text1 set string storage a:b x 1 3")
            .unwrap()
            .0,
        "data.modify"
    );
    assert_eq!(
        run("data modify storage a:b l append value [1,2]")
            .unwrap()
            .0,
        "data.modify"
    );
    assert!(run("data modify storage a:b l explode value 1").is_err());
    assert!(run("data get storage a:b {broken").is_err());
    assert!(run("data merge storage a:b 5").is_err());
    assert_eq!(run("tag @e[tag=!x] add foo").unwrap().0, "tag.add");
    assert_eq!(run("tag @s list").unwrap().0, "tag.list");
    // Selector options added for scoreboards and NBT.
    assert!(run("kill @e[scores={a=1..},team=red,nbt={x:1b},tag=t]").is_ok());
    assert!(run("kill @e[scores={a=1..]").is_err());
}

#[test]
fn smoke_commands_parse() {
    for line in [
        "tag @s list",
        "random value 1..6",
        "team list",
        "data get storage a:b",
        "scoreboard objectives list",
        "function test:a",
        "execute store result score #r t run function test:a",
        "schedule function test:a 20t",
        "datapack list",
        "bossbar list",
        "item replace entity @s hotbar.0 with minecraft:stone 3",
    ] {
        assert!(run(line).is_ok(), "{line}: {:?}", run(line).err());
    }
}

#[test]
fn execute_tree_covers_vanilla_forms() {
    for line in [
        "execute as @a at @s run say hi",
        "execute store result score @s x run data get entity @s Health",
        "execute store success bossbar minecraft:b value run say hi",
        "execute store result storage a:b out int 1.5 run time query gametime",
        "execute store result entity @s Air short 1 run say hi",
        "execute if score @s a = @s b run say hi",
        "execute unless score @s a matches ..5 run say hi",
        "execute if block ~ ~ ~ minecraft:stone",
        "execute if blocks 0 0 0 1 1 1 5 5 5 masked run say hi",
        "execute if biome ~ ~ ~ minecraft:plains",
        "execute if loaded ~ ~ ~",
        "execute if dimension minecraft:overworld",
        "execute if data entity @s Inventory[0]",
        "execute unless data storage a:b x run say hi",
        "execute if predicate a:b",
        "execute if function a:b run say hi",
        "execute if items entity @s hotbar.* minecraft:stone",
        "execute if slots block 0 0 0 container.*",
        "execute positioned over world_surface run say hi",
        "execute summon minecraft:pig run say hi",
        "execute on vehicle run say hi",
        "execute on passengers as @s run say hi",
        "execute rotated as @s facing entity @s eyes align xyz anchored feet in minecraft:overworld run say hi",
    ] {
        assert!(run(line).is_ok(), "{line}: {:?}", run(line).err());
    }
    assert!(run("execute positioned over nothing run say hi").is_err());
    assert!(run("execute on nobody run say hi").is_err());
    assert!(run("execute store result score @s").is_err());
}

#[test]
fn function_and_misc_trees() {
    assert_eq!(run("function a:b").unwrap().0, "function");
    assert_eq!(run("function #a:tag {x:1}").unwrap().0, "function");
    assert_eq!(
        run("function a:b with storage c:d path.x").unwrap().0,
        "function"
    );
    assert_eq!(run("return 5").unwrap().0, "return.value");
    assert_eq!(run("return fail").unwrap().0, "return.fail");
    assert_eq!(run("return run say hi").unwrap().0, "say");
    assert_eq!(
        run("schedule function a:b 5s append").unwrap().0,
        "schedule.function"
    );
    assert_eq!(run("schedule clear a:b").unwrap().0, "schedule.clear");
    assert_eq!(run("random roll 1..20").unwrap().0, "random.roll");
    assert_eq!(
        run("random reset * 5 true false").unwrap().0,
        "random.reset"
    );
    assert_eq!(run("teammsg hi there").unwrap().0, "teammsg");
    assert_eq!(run("tm hi").unwrap().0, "teammsg");
    assert_eq!(
        run("bossbar set a:b color pink").unwrap().0,
        "bossbar.set.color"
    );
    assert_eq!(
        run("bossbar set a:b players @a").unwrap().0,
        "bossbar.set.players"
    );
    assert_eq!(run("bossbar get a:b max").unwrap().0, "bossbar.get");
    assert_eq!(
        run("item fill block 1 2 3 container.* with minecraft:stone 5")
            .unwrap()
            .0,
        "item"
    );
    assert_eq!(
        run("item replace entity @s armor.head from block 0 0 0 container.3")
            .unwrap()
            .0,
        "item"
    );
    assert_eq!(run("tick rate 40").unwrap().0, "tick.rate");
    assert_eq!(run("tick step 1s").unwrap().0, "tick.step");
    assert_eq!(run("reload").unwrap().0, "reload");
    assert_eq!(
        run("datapack enable \"file/x\" after \"file/y\"")
            .unwrap()
            .0,
        "datapack.enable"
    );
    assert!(run("bossbar set a:b value -1").is_err());
    assert!(run("item replace entity @s nowhere.0 with minecraft:stone").is_err());
}

#[test]
fn world_command_trees() {
    for (line, id) in [
        ("recipe give @s *", "recipe.give"),
        ("recipe take @a minecraft:stick", "recipe.take"),
        ("damage @e[limit=1] 5", "damage"),
        (
            "damage @s 3.5 minecraft:magic by @e[limit=1] from @s",
            "damage",
        ),
        ("damage @s 3 minecraft:fall at ~ ~ ~", "damage"),
        ("rotate @s 90 0", "rotate"),
        ("rotate @s facing entity @e[limit=1] eyes", "rotate"),
        ("rotate @s facing 0 64 0", "rotate"),
        ("swing", "swing"),
        ("swing @e offhand whack 10", "swing"),
        ("playsound minecraft:entity.cow.ambient", "playsound"),
        (
            "playsound minecraft:block.stone.break block @s ~ ~ ~ 1 1 0.5",
            "playsound",
        ),
        ("stopsound @s", "stopsound"),
        ("stopsound @s * minecraft:ambient.cave", "stopsound"),
        ("stopsound @s music minecraft:music.game", "stopsound"),
        ("particle minecraft:smoke", "particle"),
        (
            "particle minecraft:dust{color:[1.0,0.0,0.0],scale:2.0} ~ ~ ~ 0.1 0.1 0.1 0 10 force @a",
            "particle",
        ),
        (
            "particle minecraft:block{block_state:\"minecraft:stone\"} ~ ~ ~",
            "particle",
        ),
        ("attribute @s minecraft:max_health get 2", "attribute.get"),
        (
            "attribute @s minecraft:max_health base set 40",
            "attribute.base.set",
        ),
        (
            "attribute @s minecraft:scale modifier add a:b 1.5 add_multiplied_base",
            "attribute.modifier.add",
        ),
        (
            "attribute @s minecraft:scale modifier remove a:b",
            "attribute.modifier.remove",
        ),
        (
            "attribute @s minecraft:scale modifier value get a:b 2",
            "attribute.modifier.get",
        ),
        ("ride @s mount @e[limit=1]", "ride.mount"),
        ("ride @s dismount", "ride.dismount"),
        ("spreadplayers 0 0 5 20 false @a", "spreadplayers"),
        ("spreadplayers ~ ~ 5 20 under 100 true @e", "spreadplayers"),
        ("forceload add 0 0 100 100", "forceload.add"),
        ("forceload remove all", "forceload.remove.all"),
        ("forceload query", "forceload.query"),
        ("forceload query ~ ~", "forceload.query"),
        (
            "fillbiome 0 0 0 10 10 10 minecraft:plains replace minecraft:desert",
            "fillbiome",
        ),
        ("worldborder set 100 5s", "worldborder.set"),
        ("worldborder add -10", "worldborder.add"),
        ("worldborder center 5 5", "worldborder.center"),
        ("worldborder damage amount 1", "worldborder.damage.amount"),
        ("worldborder warning time 10s", "worldborder.warning.time"),
        ("worldborder get", "worldborder.get"),
        (
            "loot give @s loot minecraft:chests/simple_dungeon",
            "loot.loot",
        ),
        ("loot spawn ~ ~ ~ kill @e[limit=1]", "loot.kill"),
        ("loot insert 0 0 0 mine 1 2 3 mainhand", "loot.mine"),
        (
            "loot replace entity @s hotbar.0 3 fish minecraft:gameplay/fishing 0 0 0",
            "loot.fish",
        ),
        (
            "loot replace block 1 2 3 container.5 loot minecraft:chests/x",
            "loot.loot",
        ),
    ] {
        assert_eq!(
            run(line).unwrap_or_else(|e| panic!("{line}: {e:?}")).0,
            id,
            "{line}"
        );
    }
    assert!(run("playsound minecraft:x invalid_source").is_err());
    assert!(run("particle minecraft:not_a_particle").is_err());
    assert!(run("damage @s -1").is_err());
    assert!(run("spreadplayers 0 0 5 0 false @a").is_err());
    assert!(run("worldborder warning distance -1").is_err());
}

#[test]
fn place_template_tree() {
    for line in [
        "place template minecraft:igloo/top",
        "place template minecraft:igloo/top ~ ~ ~",
        "place template minecraft:igloo/top 1 2 3 clockwise_90 left_right 0.5 12 strict",
    ] {
        assert_eq!(
            run(line).unwrap_or_else(|e| panic!("{line}: {e:?}")).0,
            "place"
        );
    }
    assert!(run("place template minecraft:x ~ ~ ~ sideways").is_err());
    assert!(run("place template minecraft:x ~ ~ ~ none none 2").is_err());
}
#[test]
fn mod_commands_join_the_tree_for_every_player() {
    let reg = Registries {
        permission: 0,
        ..reg()
    };
    let d = vanilla::game_dispatcher().with_mod_commands(["where"]);
    for line in ["where", "where north 12"] {
        let parse = d.parse(line, &reg);
        assert_eq!(parse.executable().unwrap().1, MOD_COMMAND, "{line}");
    }
    let parse = d.parse("w", &reg);
    let names: Vec<String> = d
        .client_suggestions(&parse, 1, &reg)
        .1
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert!(names.contains(&"where".to_owned()), "{names:?}");
    // The game's own tree is left as it was.
    assert!(vanilla::game_dispatcher().find("where").is_none());
}

#[test]
fn horizon_commands_are_off_unless_the_server_turns_them_on() {
    // By default the game runs vanilla's tree: /help lists no /pregen.
    assert!(vanilla::game_dispatcher().find("pregen").is_none());
    for dev in [false, true] {
        let vanilla_tree = vanilla::game_tree(dev, false, false);
        assert!(vanilla_tree.find("pregen").is_none());
        let d = vanilla::game_tree(dev, true, false);
        let parse = d.parse("pregen start 100 circle", &reg());
        assert_eq!(parse.executable().unwrap().1, "pregen.start");
        // Vanilla's commands are the same with or without it.
        assert_eq!(
            d.nodes[0].children.len(),
            vanilla_tree.nodes[0].children.len() + 1
        );
    }
}
/// `run` on a dedicated server's tree.
fn run_dedicated(input: &str) -> CmdResult<(String, Vec<String>)> {
    let reg = reg();
    let parse = vanilla::game_tree(true, false, true).parse(input, &reg);
    let (ctx, id) = parse.executable()?;
    Ok((
        id.to_owned(),
        ctx.args.iter().map(|a| a.name.clone()).collect(),
    ))
}
#[test]
fn profile_commands_parse_like_vanilla() {
    // OpCommand, DeOpCommands, BanPlayerCommands, PardonCommand and
    // WhitelistCommand: GameProfileArgument targets, admins only.
    let run = run_dedicated;
    assert_eq!(run("op Steve").unwrap().0, "op");
    assert_eq!(run("deop @a").unwrap().0, "deop");
    assert_eq!(run("pardon Steve").unwrap().0, "pardon");
    assert_eq!(run("ban Steve").unwrap().0, "ban");
    assert_eq!(
        run("ban Steve griefing the farm").unwrap().1,
        ["targets", "reason"]
    );
    for word in ["on", "off", "list", "reload"] {
        assert_eq!(
            run(&format!("whitelist {word}")).unwrap().0,
            format!("whitelist.{word}")
        );
    }
    assert_eq!(run("whitelist add Steve").unwrap().0, "whitelist.add");
    assert_eq!(run("whitelist remove @p").unwrap().0, "whitelist.remove");
    // GameProfileArgument: entity selectors are refused.
    assert_eq!(
        run("op @e").unwrap_err().message.plain(),
        "argument.player.entities"
    );
    assert!(run("whitelist add").is_err());
    let reg = Registries {
        permission: 2,
        ..reg()
    };
    let d = vanilla::game_tree(false, false, true);
    for line in ["op Steve", "ban Steve", "whitelist on"] {
        let e = d.parse(line, &reg).executable().unwrap_err();
        assert_eq!(e.message.plain(), "command.unknown.command", "{line}");
    }
}
#[test]
fn the_dedicated_commands_complete_from_the_servers_lists() {
    // OpCommand, DeOpCommands, PardonCommand, PardonIpCommand and WhitelistCommand: the server answers from
    // its own lists (vanilla's `.suggests(...)`, javap on the pinned 26.3 classes), not from the registries'
    // default of the one local player.
    let reg = Registries {
        permission: 4,
        players: vec!["Alex".into(), "Steve".into(), "Zed".into()],
        ops: vec!["steve".into()],
        banned_players: vec!["Griefer".into()],
        banned_ips: vec!["203.0.113.9".into()],
        whitelisted: vec!["Alex".into(), "Old".into()],
        ..reg()
    };
    let d = vanilla::game_tree(false, false, true);
    let suggest = |input: &str| -> Vec<String> {
        let parse = d.parse(input, &reg);
        d.suggestions(&parse, input.len(), &reg)
            .1
            .into_iter()
            .map(|s| s.text)
            .collect()
    };
    // Only the online players who are not operators (the match ignores case).
    assert_eq!(suggest("op "), ["Alex", "Zed"]);
    assert_eq!(suggest("op z"), ["Zed"]);
    assert_eq!(suggest("deop "), ["steve"]);
    assert_eq!(suggest("pardon "), ["Griefer"]);
    assert_eq!(suggest("pardon-ip "), ["203.0.113.9"]);
    // Add offers the online players who are not whitelisted; remove offers the list, online or not.
    assert_eq!(suggest("whitelist add "), ["Steve", "Zed"]);
    assert_eq!(suggest("whitelist remove "), ["Alex", "Old"]);
    // Nothing else changed: ban still completes by the argument type, with the selectors.
    assert!(suggest("ban ").contains(&"@a".to_owned()));
}
#[test]
fn only_a_dedicated_server_has_the_dedicated_commands() {
    // Commands.CommandSelection: an integrated server (single player, a LAN
    // world) registers no /op, /ban, /ban-ip, /banlist, /pardon, /pardon-ip,
    // /deop or /whitelist, so /help and Tab don't offer them.
    let dedicated = [
        "ban",
        "ban-ip",
        "banlist",
        "deop",
        "op",
        "pardon",
        "pardon-ip",
        "whitelist",
    ];
    for dev in [false, true] {
        for horizon in [false, true] {
            let integrated = vanilla::game_tree(dev, horizon, false);
            let server = vanilla::game_tree(dev, horizon, true);
            for name in dedicated {
                assert!(integrated.find(name).is_none(), "{name}");
                assert!(server.find(name).is_some(), "{name}");
            }
            // includeIntegrated: /publish and /unpublish are the integrated
            // server's alone.
            for name in ["publish", "unpublish"] {
                assert!(integrated.find(name).is_some(), "{name}");
                assert!(server.find(name).is_none(), "{name}");
            }
            assert_eq!(
                server.nodes[0].children.len(),
                integrated.nodes[0].children.len() + dedicated.len() - 2
            );
        }
    }
    assert!(vanilla::game_dispatcher().find("ban").is_none());
    assert!(vanilla::release_dispatcher().find("ban").is_none());
}
/// The root literals a player at `level` is offered (the chat's, client tree).
fn root_commands(d: &Dispatcher, level: u8) -> Vec<String> {
    let reg = Registries {
        permission: level,
        ..reg()
    };
    let parse = d.parse("", &reg);
    d.client_suggestions(&parse, 0, &reg)
        .1
        .into_iter()
        .map(|s| s.text)
        .collect()
}
#[test]
fn a_dedicated_server_keeps_seed_and_version_for_game_masters() {
    // Commands.java: SeedCommand.register(dispatcher, selection != INTEGRATED)
    // and VersionCommand likewise; `checkPermissions` makes them require
    // LEVEL_GAMEMASTERS instead of LEVEL_ALL. A dedicated server's player
    // without op gets neither (a seed cracker's input on an SMP).
    for dev in [false, true] {
        for horizon in [false, true] {
            let integrated = vanilla::game_tree(dev, horizon, false);
            let server = vanilla::game_tree(dev, horizon, true);
            let open = root_commands(integrated, 0);
            assert!(open.contains(&"seed".to_owned()), "{open:?}");
            assert!(open.contains(&"version".to_owned()), "{open:?}");
            assert_eq!(
                root_commands(server, 0),
                [
                    "help", "list", "me", "msg", "random", "teammsg", "tell", "tm", "trigger", "w"
                ]
            );
            let gm = root_commands(server, 2);
            assert!(gm.contains(&"seed".to_owned()), "{gm:?}");
            assert!(gm.contains(&"version".to_owned()), "{gm:?}");
            for line in ["seed", "version"] {
                let player = Registries {
                    permission: 0,
                    ..reg()
                };
                // Brigadier: a node the source can't use is no node at all.
                let e = server.parse(line, &player).executable().unwrap_err();
                assert_eq!(e.message.plain(), "command.unknown.command", "{line}");
                assert_eq!(e.input.as_deref(), Some(line), "{line}");
                assert!(integrated.parse(line, &player).executable().is_ok());
                let op = Registries {
                    permission: 2,
                    ..reg()
                };
                assert_eq!(server.parse(line, &op).executable().unwrap().1, line);
            }
        }
    }
}
