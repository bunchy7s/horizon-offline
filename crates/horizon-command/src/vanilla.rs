//! The vanilla command tree (Commands.java registrations) for the commands
//! Horizon implements. Leaves carry command ids that the game executes;
//! mode keywords are read back from the parsed literals.
use super::{
    Dispatcher,
    args::{ArgType, Resource, integer},
};
use horizon_client_core::gamerules::{RULES, Value};

/// Permission levels (Commands.LEVEL_*).
pub const ALL: u8 = 0;
pub const GAMEMASTERS: u8 = 2;

pub const PLAYERS: ArgType = ArgType::Entity {
    single: false,
    players: true,
};
pub const PLAYER: ArgType = ArgType::Entity {
    single: true,
    players: true,
};
pub const ENTITIES: ArgType = ArgType::Entity {
    single: false,
    players: false,
};
pub const ENTITY: ArgType = ArgType::Entity {
    single: true,
    players: false,
};

pub const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

pub fn build() -> Dispatcher {
    build_with(true)
}
/// The tree Commands registers; `dev` adds the commands vanilla only
/// registers in development builds (raid, warden_spawn_tracker).
pub fn build_with(dev: bool) -> Dispatcher {
    build_for(dev, false)
}
/// [`build_with`] for a server of the given kind (Commands.CommandSelection):
/// an integrated server's tree, or with `dedicated` a dedicated server's.
pub fn build_for(dev: bool, dedicated: bool) -> Dispatcher {
    let mut d = registered(dev);
    if dedicated {
        dedicated_selection(&mut d);
    }
    apply_vanilla_order(&mut d);
    d
}
/// Commands(CommandSelection.DEDICATED) where it differs from INTEGRATED:
/// `seed` and `version` check permissions (SeedCommand.register and
/// VersionCommand.register get `selection != INTEGRATED`), `publish` and
/// `unpublish` are not registered (`includeIntegrated`), and the dedicated
/// commands are (`includeDedicated`: `/op`, `/ban`, `/whitelist`...).
fn dedicated_selection(d: &mut Dispatcher) {
    for name in ["seed", "version"] {
        if let Some(n) = d.find(name) {
            d.requires(n, GAMEMASTERS);
        }
    }
    let kept: Vec<usize> = d.nodes[0]
        .children
        .iter()
        .copied()
        .filter(|&c| !matches!(d.nodes[c].name(), "publish" | "unpublish"))
        .collect();
    d.nodes[0].children = kept;
    super::tree_extra::register_dedicated(d, 0);
}
/// The tree in the order Horizon's code registers it, before
/// [`vanilla_order`](super::vanilla_order) puts children in vanilla's order
/// (tools/generate_command_order.py compares the two).
pub fn registered(dev: bool) -> Dispatcher {
    let mut d = Dispatcher::new();
    let root = 0;

    advancement(&mut d, root);

    // help [<command>]
    let help = d.literal(root, "help");
    d.executes(help, "help");
    let c = d.argument(help, "command", ArgType::Greedy);
    d.executes(c, "help");

    // gamemode <gamemode> [<target>]
    let gm = d.literal(root, "gamemode");
    d.requires(gm, GAMEMASTERS);
    let mode = d.argument(gm, "gamemode", ArgType::GameMode);
    d.executes(mode, "gamemode");
    let t = d.argument(mode, "target", PLAYERS);
    d.executes(t, "gamemode");

    let dgm = d.literal(root, "defaultgamemode");
    d.requires(dgm, GAMEMASTERS);
    let mode = d.argument(dgm, "gamemode", ArgType::GameMode);
    d.executes(mode, "defaultgamemode");

    // give <targets> <item> [<count>]
    let give = d.literal(root, "give");
    d.requires(give, GAMEMASTERS);
    let targets = d.argument(give, "targets", PLAYERS);
    let item = d.argument(targets, "item", ArgType::Item);
    d.executes(item, "give");
    let count = d.argument(item, "count", integer(1));
    d.executes(count, "give");

    // teleport / tp
    let tp = d.literal(root, "teleport");
    d.requires(tp, GAMEMASTERS);
    let loc = d.argument(tp, "location", ArgType::Vec3 { center: true });
    d.executes(loc, "teleport.location");
    let dest = d.argument(tp, "destination", ENTITY);
    d.executes(dest, "teleport.destination");
    let targets = d.argument(tp, "targets", ENTITIES);
    let loc = d.argument(targets, "location", ArgType::Vec3 { center: true });
    d.executes(loc, "teleport.targets.location");
    let rot = d.argument(loc, "rotation", ArgType::Rotation);
    d.executes(rot, "teleport.targets.location");
    let facing = d.literal(loc, "facing");
    let fe = d.literal(facing, "entity");
    let fent = d.argument(fe, "facingEntity", ENTITY);
    d.executes(fent, "teleport.targets.location");
    let anchor = d.argument(fent, "facingAnchor", ArgType::Anchor);
    d.executes(anchor, "teleport.targets.location");
    let floc = d.argument(facing, "facingLocation", ArgType::Vec3 { center: true });
    d.executes(floc, "teleport.targets.location");
    let dest = d.argument(targets, "destination", ENTITY);
    d.executes(dest, "teleport.targets.destination");
    let alias = d.literal(root, "tp");
    d.requires(alias, GAMEMASTERS);
    d.redirect(alias, tp);

    // time: clock nodes for the dimension's default clock, plus `of <clock>`.
    let time = d.literal(root, "time");
    d.requires(time, GAMEMASTERS);
    clock_nodes(&mut d, time);
    let query = d.literal(time, "query");
    let gt = d.literal(query, "gametime");
    d.executes(gt, "time.query.gametime");
    let of = d.literal(time, "of");
    let clock = d.argument(of, "clock", ArgType::Resource(Resource::Clock));
    clock_nodes(&mut d, clock);

    // weather (clear|rain|thunder) [<duration>]
    let weather = d.literal(root, "weather");
    d.requires(weather, GAMEMASTERS);
    for kind in ["clear", "rain", "thunder"] {
        let k = d.literal(weather, kind);
        d.executes(k, "weather");
        let dur = d.argument(k, "duration", ArgType::Time { min: 1 });
        d.executes(dur, "weather");
    }

    if dev {
        // raid start <omenlvl> | stop | check | sound <type> | spawnleader | setomen <level> | glow
        let raid = d.literal(root, "raid");
        d.requires(raid, 3);
        let start = d.literal(raid, "start");
        let level = d.argument(start, "omenlvl", integer(0));
        d.executes(level, "raid.start");
        for (name, id) in [
            ("stop", "raid.stop"),
            ("check", "raid.check"),
            ("spawnleader", "raid.spawnleader"),
            ("glow", "raid.glow"),
        ] {
            let n = d.literal(raid, name);
            d.executes(n, id);
        }
        let sound = d.literal(raid, "sound");
        let kind = d.argument(sound, "type", ArgType::Greedy);
        d.executes(kind, "raid.sound");
        let setomen = d.literal(raid, "setomen");
        let level = d.argument(setomen, "level", integer(0));
        d.executes(level, "raid.setomen");
    }

    // kill [<targets>]
    let kill = d.literal(root, "kill");
    d.requires(kill, GAMEMASTERS);
    d.executes(kill, "kill");
    let t = d.argument(kill, "targets", ENTITIES);
    d.executes(t, "kill");

    // effect clear [<targets> [<effect>]] | give <targets> <effect> [<seconds>|infinite [<amplifier> [<hideParticles>]]]
    let effect = d.literal(root, "effect");
    d.requires(effect, GAMEMASTERS);
    let clear = d.literal(effect, "clear");
    d.executes(clear, "effect.clear");
    let t = d.argument(clear, "targets", ENTITIES);
    d.executes(t, "effect.clear");
    let e = d.argument(t, "effect", ArgType::Resource(Resource::Effect));
    d.executes(e, "effect.clear");
    let give = d.literal(effect, "give");
    let t = d.argument(give, "targets", ENTITIES);
    let e = d.argument(t, "effect", ArgType::Resource(Resource::Effect));
    d.executes(e, "effect.give");
    let seconds = d.argument(
        e,
        "seconds",
        ArgType::Integer {
            min: 1,
            max: 1_000_000,
        },
    );
    let infinite = d.literal(e, "infinite");
    for duration in [seconds, infinite] {
        d.executes(duration, "effect.give");
        let amp = d.argument(duration, "amplifier", ArgType::Integer { min: 0, max: 255 });
        d.executes(amp, "effect.give");
        let hide = d.argument(amp, "hideParticles", ArgType::Bool);
        d.executes(hide, "effect.give");
    }

    // enchant <targets> <enchantment> [<level>]
    let enchant = d.literal(root, "enchant");
    d.requires(enchant, GAMEMASTERS);
    let t = d.argument(enchant, "targets", ENTITIES);
    let e = d.argument(t, "enchantment", ArgType::Resource(Resource::Enchantment));
    d.executes(e, "enchant");
    let level = d.argument(e, "level", integer(0));
    d.executes(level, "enchant");

    // summon <entity> [<pos>] [<nbt>]
    let summon = d.literal(root, "summon");
    d.requires(summon, GAMEMASTERS);
    let e = d.argument(summon, "entity", ArgType::Resource(Resource::EntityType));
    d.suggests(e, super::Suggests::SummonableEntities);
    d.executes(e, "summon");
    let pos = d.argument(e, "pos", ArgType::Vec3 { center: true });
    d.executes(pos, "summon");
    let nbt = d.argument(pos, "nbt", ArgType::Nbt);
    d.executes(nbt, "summon");

    // setblock <pos> <block> [destroy|keep|replace|strict]
    let sb = d.literal(root, "setblock");
    d.requires(sb, GAMEMASTERS);
    let pos = d.argument(sb, "pos", ArgType::BlockPos);
    let block = d.argument(pos, "block", ArgType::BlockState);
    d.executes(block, "setblock");
    for m in ["destroy", "keep", "replace", "strict"] {
        let l = d.literal(block, m);
        d.executes(l, "setblock");
    }

    // fill <from> <to> <block> [modes]
    let fill = d.literal(root, "fill");
    d.requires(fill, GAMEMASTERS);
    let from = d.argument(fill, "from", ArgType::BlockPos);
    let to = d.argument(from, "to", ArgType::BlockPos);
    let block = d.argument(to, "block", ArgType::BlockState);
    fill_modes(&mut d, block);
    let replace = d.literal(block, "replace");
    d.executes(replace, "fill");
    let filter = d.argument(replace, "filter", ArgType::BlockPredicate);
    fill_modes(&mut d, filter);
    let keep = d.literal(block, "keep");
    d.executes(keep, "fill");

    // clone [from <dim>] <begin> <end> [to <dim>] <destination> [strict] [mask] [mode]
    let clone = d.literal(root, "clone");
    d.requires(clone, GAMEMASTERS);
    clone_begin(&mut d, clone);
    let from = d.literal(clone, "from");
    let dim = d.argument(from, "sourceDimension", ArgType::Dimension);
    clone_begin(&mut d, dim);

    // experience / xp
    let xp = d.literal(root, "experience");
    d.requires(xp, GAMEMASTERS);
    for (op, min) in [("add", i32::MIN), ("set", 0)] {
        let o = d.literal(xp, op);
        let t = d.argument(o, "target", PLAYERS);
        let amount = d.argument(t, "amount", integer(min));
        d.executes(amount, "experience");
        for unit in ["points", "levels"] {
            let u = d.literal(amount, unit);
            d.executes(u, "experience");
        }
    }
    let q = d.literal(xp, "query");
    let t = d.argument(q, "target", PLAYER);
    for unit in ["points", "levels"] {
        let u = d.literal(t, unit);
        d.executes(u, "experience");
    }
    let alias = d.literal(root, "xp");
    d.requires(alias, GAMEMASTERS);
    d.redirect(alias, xp);

    // gamerule <rule> [<value>] (short and namespaced names)
    let gr = d.literal(root, "gamerule");
    d.requires(gr, GAMEMASTERS);
    for rule in RULES {
        for name in [rule.name.to_owned(), format!("minecraft:{}", rule.name)] {
            let r = d.literal(gr, &name);
            d.executes(r, "gamerule");
            let ty = match rule.default {
                Value::Bool(_) => ArgType::Bool,
                Value::Int(_) => ArgType::Integer {
                    min: rule.min,
                    max: rule.max,
                },
            };
            let v = d.argument(r, "value", ty);
            d.executes(v, "gamerule");
        }
    }

    // difficulty [<difficulty>]
    let diff = d.literal(root, "difficulty");
    d.requires(diff, GAMEMASTERS);
    d.executes(diff, "difficulty");
    for name in DIFFICULTIES {
        let l = d.literal(diff, name);
        d.executes(l, "difficulty");
    }

    // seed and version: everyone in the integrated server, game masters on a
    // dedicated one (`dedicated_selection`).
    let seed = d.literal(root, "seed");
    d.executes(seed, "seed");
    let version = d.literal(root, "version");
    d.executes(version, "version");

    // spawnpoint [<targets>] [<pos>] [<rotation>]
    let sp = d.literal(root, "spawnpoint");
    d.requires(sp, GAMEMASTERS);
    d.executes(sp, "spawnpoint");
    let t = d.argument(sp, "targets", PLAYERS);
    d.executes(t, "spawnpoint");
    let pos = d.argument(t, "pos", ArgType::BlockPos);
    d.executes(pos, "spawnpoint");
    let rot = d.argument(pos, "rotation", ArgType::Rotation);
    d.executes(rot, "spawnpoint");

    // setworldspawn [<pos>] [<rotation>]
    let ws = d.literal(root, "setworldspawn");
    d.requires(ws, GAMEMASTERS);
    d.executes(ws, "setworldspawn");
    let pos = d.argument(ws, "pos", ArgType::BlockPos);
    d.executes(pos, "setworldspawn");
    let rot = d.argument(pos, "rotation", ArgType::Rotation);
    d.executes(rot, "setworldspawn");

    // clear [<targets>] [<item>] [<maxCount>]
    let clear = d.literal(root, "clear");
    d.requires(clear, GAMEMASTERS);
    d.executes(clear, "clear");
    let t = d.argument(clear, "targets", PLAYERS);
    d.executes(t, "clear");
    let item = d.argument(t, "item", ArgType::ItemPredicate);
    d.executes(item, "clear");
    let max = d.argument(item, "maxCount", integer(0));
    d.executes(max, "clear");

    // locate (structure|biome) <...>
    let locate = d.literal(root, "locate");
    d.requires(locate, GAMEMASTERS);
    let s = d.literal(locate, "structure");
    let a = d.argument(
        s,
        "structure",
        ArgType::ResOrTagKey("minecraft:worldgen/structure"),
    );
    d.executes(a, "locate.structure");
    let b = d.literal(locate, "biome");
    let a = d.argument(b, "biome", ArgType::ResOrTag("minecraft:worldgen/biome"));
    d.executes(a, "locate.biome");
    let p = d.literal(locate, "poi");
    let a = d.argument(
        p,
        "poi",
        ArgType::ResOrTag("minecraft:point_of_interest_type"),
    );
    d.executes(a, "locate.poi");

    if dev {
        // warden_spawn_tracker (clear | set <warning_level>)
        let tracker = d.literal(root, "warden_spawn_tracker");
        d.requires(tracker, GAMEMASTERS);
        let clear = d.literal(tracker, "clear");
        d.executes(clear, "warden_spawn_tracker.clear");
        let set = d.literal(tracker, "set");
        let level = d.argument(set, "warning_level", ArgType::Integer { min: 0, max: 4 });
        d.executes(level, "warden_spawn_tracker.set");
    }

    // say / me / msg / list
    let say = d.literal(root, "say");
    d.requires(say, GAMEMASTERS);
    let m = d.argument(say, "message", ArgType::Message);
    d.executes(m, "say");
    let me = d.literal(root, "me");
    let m = d.argument(me, "action", ArgType::Message);
    d.executes(m, "me");
    let msg = d.literal(root, "msg");
    let t = d.argument(msg, "targets", PLAYERS);
    let m = d.argument(t, "message", ArgType::Message);
    d.executes(m, "msg");
    for alias in ["tell", "w"] {
        let a = d.literal(root, alias);
        d.redirect(a, msg);
    }
    let list = d.literal(root, "list");
    d.executes(list, "list");
    let uuids = d.literal(list, "uuids");
    d.executes(uuids, "list");

    // tellraw <targets> <message>
    let tellraw = d.literal(root, "tellraw");
    d.requires(tellraw, GAMEMASTERS);
    let t = d.argument(tellraw, "targets", PLAYERS);
    let m = d.argument(t, "message", ArgType::Component);
    d.executes(m, "tellraw");

    // title <targets> clear|reset|title|subtitle|actionbar <title>|times <in> <stay> <out>
    let title = d.literal(root, "title");
    d.requires(title, GAMEMASTERS);
    let t = d.argument(title, "targets", PLAYERS);
    for mode in ["clear", "reset"] {
        let l = d.literal(t, mode);
        d.executes(l, "title");
    }
    for mode in ["title", "subtitle", "actionbar"] {
        let l = d.literal(t, mode);
        let c = d.argument(l, "title", ArgType::Component);
        d.executes(c, "title");
    }
    let times = d.literal(t, "times");
    let fade_in = d.argument(times, "fadeIn", ArgType::Time { min: 0 });
    let stay = d.argument(fade_in, "stay", ArgType::Time { min: 0 });
    let fade_out = d.argument(stay, "fadeOut", ArgType::Time { min: 0 });
    d.executes(fade_out, "title");

    super::tree_scoreboard::register(&mut d, root);
    super::tree_data::register(&mut d, root);
    super::tree_execute::register(&mut d, root);
    super::tree_misc::register(&mut d, root);
    super::tree_world::register(&mut d, root);
    super::tree_extra::register(&mut d, root);
    d
}

/// AdvancementCommands: grant|revoke <targets> only|from|until|through
/// <advancement> | everything; `only` takes an optional criterion.
fn advancement(d: &mut Dispatcher, root: usize) {
    let adv = d.literal(root, "advancement");
    d.requires(adv, GAMEMASTERS);
    for action in ["grant", "revoke"] {
        let a = d.literal(adv, action);
        let t = d.argument(a, "targets", PLAYERS);
        for mode in ["only", "from", "until", "through"] {
            let m = d.literal(t, mode);
            let id = d.argument(m, "advancement", ArgType::ResKey("minecraft:advancement"));
            d.executes(id, "advancement");
            if mode == "only" {
                let c = d.argument(id, "criterion", ArgType::Greedy);
                d.suggests(c, super::Suggests::AdvancementCriteria);
                d.executes(c, "advancement");
            }
        }
        let e = d.literal(t, "everything");
        d.executes(e, "advancement");
    }
}

/// TimeCommand.addClockNodes.
fn clock_nodes(d: &mut Dispatcher, node: usize) {
    let set = d.literal(node, "set");
    let t = d.argument(set, "time", ArgType::Time { min: 0 });
    d.executes(t, "time.set");
    let marker = d.argument(set, "timemarker", ArgType::Id("time_markers"));
    d.suggests(marker, super::Suggests::TimeMarkers);
    d.executes(marker, "time.set.marker");
    let add = d.literal(node, "add");
    let t = d.argument(add, "time", ArgType::Time { min: i32::MIN });
    d.executes(t, "time.add");
    let pause = d.literal(node, "pause");
    d.executes(pause, "time.pause");
    let resume = d.literal(node, "resume");
    d.executes(resume, "time.resume");
    let rate = d.literal(node, "rate");
    let r = d.argument(
        rate,
        "rate",
        ArgType::Float {
            min: 1.0e-5,
            max: 1000.0,
        },
    );
    d.executes(r, "time.rate");
    let query = d.literal(node, "query");
    let time = d.literal(query, "time");
    d.executes(time, "time.query.time");
    let tl = d.argument(query, "timeline", ArgType::Resource(Resource::Timeline));
    d.executes(tl, "time.query.timeline");
    let rep = d.literal(tl, "repetition");
    d.executes(rep, "time.query.repetition");
}
/// FillCommand.wrapWithMode.
fn fill_modes(d: &mut Dispatcher, node: usize) {
    d.executes(node, "fill");
    for m in ["outline", "hollow", "destroy", "strict"] {
        let l = d.literal(node, m);
        d.executes(l, "fill");
    }
}
fn clone_begin(d: &mut Dispatcher, node: usize) {
    let begin = d.argument(node, "begin", ArgType::BlockPos);
    let end = d.argument(begin, "end", ArgType::BlockPos);
    clone_destination(d, end);
    let to = d.literal(end, "to");
    let dim = d.argument(to, "targetDimension", ArgType::Dimension);
    clone_destination(d, dim);
}
fn clone_destination(d: &mut Dispatcher, node: usize) {
    let dest = d.argument(node, "destination", ArgType::BlockPos);
    clone_mask(d, dest);
    let strict = d.literal(dest, "strict");
    clone_mask(d, strict);
}
fn clone_mask(d: &mut Dispatcher, node: usize) {
    d.executes(node, "clone");
    for m in ["replace", "masked"] {
        let l = d.literal(node, m);
        clone_mode(d, l);
    }
    let filtered = d.literal(node, "filtered");
    let filter = d.argument(filtered, "filter", ArgType::BlockPredicate);
    clone_mode(d, filter);
}
fn clone_mode(d: &mut Dispatcher, node: usize) {
    d.executes(node, "clone");
    for m in ["force", "move", "normal"] {
        let l = d.literal(node, m);
        d.executes(l, "clone");
    }
}
/// The shared vanilla tree.
/// Reorder node children as vanilla registers them (Brigadier keeps
/// children in registration order; smart usage lists them that way).
fn apply_vanilla_order(d: &mut Dispatcher) {
    for (path, order) in super::vanilla_order::ORDER {
        let mut node = Some(0usize);
        for name in path.split(' ').filter(|n| !n.is_empty()) {
            node = node.and_then(|n| {
                d.nodes[n]
                    .children
                    .iter()
                    .copied()
                    .find(|&c| d.nodes[c].name() == name)
            });
        }
        let Some(n) = node else { continue };
        let mut children = d.nodes[n].children.clone();
        let rank = |c: usize| {
            order
                .iter()
                .position(|o| *o == d.nodes[c].name())
                .unwrap_or(order.len())
        };
        children.sort_by_key(|&c| rank(c));
        d.nodes[n].children = children;
    }
}
/// The tree the game runs: a release integrated server's, or with
/// `HORIZON_DEV_COMMANDS` set the development one
/// (SharedConstants.DEBUG_DEV_COMMANDS), plus Horizon's own commands when
/// the server turned them on (`set_horizon_commands`).
pub fn game_dispatcher() -> &'static Dispatcher {
    game_tree(
        std::env::var_os("HORIZON_DEV_COMMANDS").is_some(),
        HORIZON_COMMANDS.load(std::sync::atomic::Ordering::Relaxed),
        DEDICATED_COMMANDS.load(std::sync::atomic::Ordering::Relaxed),
    )
}
/// Whether this process is a dedicated server, whose tree has the dedicated
/// commands (`/op`, `/ban`, `/whitelist`...). Single player's server thread
/// and a LAN world are integrated servers: vanilla registers none of them.
static DEDICATED_COMMANDS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// A dedicated server turns its commands on, for the trees `game_dispatcher`
/// returns from now on.
pub fn set_dedicated_commands(on: bool) {
    DEDICATED_COMMANDS.store(on, std::sync::atomic::Ordering::Relaxed);
}
/// Whether this process's server has Horizon's own commands (`/pregen`). Off
/// by default, so `/help` and the command tree are vanilla's; a dedicated
/// server turns them on with `horizon-pregen=true` in server.properties, the
/// game with `--pregen` (docs/pregeneration.md).
static HORIZON_COMMANDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Turns Horizon's own commands on or off for the trees `game_dispatcher`
/// returns from now on.
pub fn set_horizon_commands(on: bool) {
    HORIZON_COMMANDS.store(on, std::sync::atomic::Ordering::Relaxed);
}
/// The server.properties key that turns `/pregen` on (Horizon's own; vanilla
/// ignores keys it doesn't know).
pub const PREGEN_PROPERTY: &str = "horizon-pregen";
pub(crate) fn game_tree(dev: bool, horizon: bool, dedicated: bool) -> &'static Dispatcher {
    static TREES: [std::sync::OnceLock<Dispatcher>; 8] = [const { std::sync::OnceLock::new() }; 8];
    match (dev, horizon, dedicated) {
        (true, false, false) => dispatcher(),
        (false, false, false) => release_dispatcher(),
        _ => TREES[usize::from(dev) | usize::from(horizon) << 1 | usize::from(dedicated) << 2]
            .get_or_init(|| {
                let mut d = build_for(dev, dedicated);
                if horizon {
                    horizon_commands(&mut d, 0);
                }
                d
            }),
    }
}
/// Commands vanilla doesn't have, after its own: `/pregen`
/// (docs/pregeneration.md).
fn horizon_commands(d: &mut Dispatcher, root: usize) {
    let pregen = d.literal(root, "pregen");
    d.requires(pregen, GAMEMASTERS);
    let start = d.literal(pregen, "start");
    let radius = d.argument(
        start,
        "radius",
        ArgType::Integer {
            min: 0,
            max: 30_000_000,
        },
    );
    d.executes(radius, "pregen.start");
    for shape in ["square", "circle"] {
        let shape = d.literal(radius, shape);
        d.executes(shape, "pregen.start");
        let center = d.argument(shape, "center", ArgType::ColumnPos);
        d.executes(center, "pregen.start");
    }
    for (word, id) in [
        ("pause", "pregen.pause"),
        ("continue", "pregen.continue"),
        ("cancel", "pregen.cancel"),
        ("status", "pregen.status"),
    ] {
        let w = d.literal(pregen, word);
        d.executes(w, id);
    }
}
pub fn dispatcher() -> &'static Dispatcher {
    static TREE: std::sync::OnceLock<Dispatcher> = std::sync::OnceLock::new();
    TREE.get_or_init(build)
}
/// The tree of a release integrated server (no development commands).
pub fn release_dispatcher() -> &'static Dispatcher {
    static TREE: std::sync::OnceLock<Dispatcher> = std::sync::OnceLock::new();
    TREE.get_or_init(|| build_with(false))
}
