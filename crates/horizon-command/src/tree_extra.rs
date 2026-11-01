//! Commands of the hosted game rather than the world: `/dialog`, `/kick`,
//! `/posteffect`, `/fetchprofile`, `/spectate`, `/publish`, `/unpublish`,
//! `/waypoint`, `/debug`, `/jfr` and `/test`; on a dedicated server only,
//! `/op`, `/deop`, `/ban`, `/ban-ip`, `/banlist`, `/pardon`, `/pardon-ip` and
//! `/whitelist`.
use super::{
    Dispatcher, Suggests,
    args::ArgType,
    vanilla::{ENTITY, GAMEMASTERS, PLAYER, PLAYERS},
};

const ADMINS: u8 = 3;
const OWNERS: u8 = 4;

pub fn register(d: &mut Dispatcher, root: usize) {
    // DialogCommand
    let dialog = d.literal(root, "dialog");
    d.requires(dialog, GAMEMASTERS);
    let show = d.literal(dialog, "show");
    let t = d.argument(show, "targets", PLAYERS);
    let dl = d.argument(t, "dialog", ArgType::ResourceOrId("minecraft:dialog"));
    d.executes(dl, "dialog.show");
    let clear = d.literal(dialog, "clear");
    let t = d.argument(clear, "targets", PLAYERS);
    d.executes(t, "dialog.clear");

    // KickCommand
    let kick = d.literal(root, "kick");
    d.requires(kick, ADMINS);
    let t = d.argument(kick, "targets", PLAYERS);
    d.executes(t, "kick");
    let reason = d.argument(t, "reason", ArgType::Message);
    d.executes(reason, "kick");

    // PostEffectCommand
    let pe = d.literal(root, "posteffect");
    d.requires(pe, GAMEMASTERS);
    let add = d.literal(pe, "add");
    let t = d.argument(add, "targets", PLAYERS);
    let id = d.argument(t, "posteffect", ArgType::Id("post_effects"));
    d.executes(id, "posteffect.add");
    let clear = d.literal(pe, "clear");
    let t = d.argument(clear, "targets", PLAYERS);
    d.executes(t, "posteffect.clear");
    let list = d.literal(pe, "list");
    let t = d.argument(list, "target", PLAYER);
    d.executes(t, "posteffect.list");
    let remove = d.literal(pe, "remove");
    let t = d.argument(remove, "targets", PLAYERS);
    let id = d.argument(t, "posteffect", ArgType::Id("post_effects"));
    d.executes(id, "posteffect.remove");

    // FetchProfileCommand
    let fp = d.literal(root, "fetchprofile");
    d.requires(fp, GAMEMASTERS);
    let name = d.literal(fp, "name");
    let n = d.argument(name, "name", ArgType::Greedy);
    d.executes(n, "fetchprofile");
    let id = d.literal(fp, "id");
    let u = d.argument(id, "id", ArgType::Uuid);
    d.executes(u, "fetchprofile");
    let entity = d.literal(fp, "entity");
    let e = d.argument(entity, "entity", ENTITY);
    d.executes(e, "fetchprofile");

    // SpectateCommand
    let spectate = d.literal(root, "spectate");
    d.requires(spectate, GAMEMASTERS);
    d.executes(spectate, "spectate");
    let target = d.argument(spectate, "target", ENTITY);
    d.executes(target, "spectate");
    let player = d.argument(target, "player", PLAYER);
    d.executes(player, "spectate");

    // PublishCommand / UnpublishCommand
    let publish = d.literal(root, "publish");
    d.requires(publish, OWNERS);
    d.executes(publish, "publish");
    let allow = d.argument(publish, "allowCommands", ArgType::Bool);
    d.executes(allow, "publish");
    let port = d.argument(allow, "port", ArgType::Integer { min: 0, max: 65535 });
    d.executes(port, "publish");
    let unpublish = d.literal(root, "unpublish");
    d.requires(unpublish, OWNERS);
    d.executes(unpublish, "unpublish");

    // WaypointCommand
    let wp = d.literal(root, "waypoint");
    d.requires(wp, GAMEMASTERS);
    let list = d.literal(wp, "list");
    d.executes(list, "waypoint.list");
    let modify = d.literal(wp, "modify");
    let w = d.argument(modify, "waypoint", ENTITY);
    let color = d.literal(w, "color");
    let c = d.argument(color, "color", ArgType::TeamColor);
    d.executes(c, "waypoint.modify");
    let hex = d.literal(color, "hex");
    let h = d.argument(hex, "color", ArgType::HexColor);
    d.executes(h, "waypoint.modify");
    let reset = d.literal(color, "reset");
    d.executes(reset, "waypoint.modify");
    let style = d.literal(w, "style");
    let reset = d.literal(style, "reset");
    d.executes(reset, "waypoint.modify");
    let set = d.literal(style, "set");
    let st = d.argument(set, "style", ArgType::Id("none"));
    d.executes(st, "waypoint.modify");

    // DebugCommand
    let debug = d.literal(root, "debug");
    d.requires(debug, ADMINS);
    for name in ["start", "stop"] {
        let l = d.literal(debug, name);
        d.executes(l, "debug");
    }
    let function = d.literal(debug, "function");
    d.requires(function, ADMINS);
    let n = d.argument(function, "name", ArgType::Function);
    d.suggests(n, super::Suggests::Functions);
    d.executes(n, "debug");

    // JfrCommand (registered when the JVM profiler is available)
    let jfr = d.literal(root, "jfr");
    d.requires(jfr, OWNERS);
    for name in ["start", "stop"] {
        let l = d.literal(jfr, name);
        d.executes(l, "jfr");
    }

    // TestCommand (release build: no export subcommands)
    let test = d.literal(root, "test");
    d.requires(test, GAMEMASTERS);
    let tests = ArgType::ResourceSelector("minecraft:test_instance");
    let int0 = ArgType::Integer {
        min: 0,
        max: i32::MAX,
    };
    let int = ArgType::Integer {
        min: i32::MIN,
        max: i32::MAX,
    };
    // numberOfTimes [untilFailed [rotationSteps [testsPerRow]]], or only
    // the first two.
    let retry = |d: &mut Dispatcher, node: usize, id: &'static str, build_info: bool| {
        let n = d.argument(node, "numberOfTimes", int0.clone());
        d.executes(n, id);
        let u = d.argument(n, "untilFailed", ArgType::Bool);
        d.executes(u, id);
        if build_info {
            let r = d.argument(u, "rotationSteps", int.clone());
            d.executes(r, id);
            let t = d.argument(r, "testsPerRow", int.clone());
            d.executes(t, id);
        }
    };
    let run = d.literal(test, "run");
    let t = d.argument(run, "tests", tests.clone());
    d.executes(t, "test.run");
    retry(d, t, "test.run", true);
    let multiple = d.literal(test, "runmultiple");
    let t = d.argument(multiple, "tests", tests.clone());
    d.executes(t, "test.run");
    let a = d.argument(t, "amount", int.clone());
    d.executes(a, "test.run");
    for name in ["runthese", "runclosest", "runthat"] {
        let l = d.literal(test, name);
        d.executes(l, "test.run.found");
        retry(d, l, "test.run.found", false);
    }
    let failed = d.literal(test, "runfailed");
    d.executes(failed, "test.run.found");
    let only = d.argument(failed, "onlyRequiredTests", ArgType::Bool);
    d.executes(only, "test.run.found");
    retry(d, only, "test.run.found", true);
    retry(d, failed, "test.run.found", true);
    let verify = d.literal(test, "verify");
    let t = d.argument(verify, "tests", tests.clone());
    d.executes(t, "test.verify");
    let locate = d.literal(test, "locate");
    let t = d.argument(locate, "tests", tests);
    d.executes(t, "test.locate");
    for name in [
        "resetclosest",
        "resetthese",
        "resetthat",
        "clearthat",
        "clearthese",
    ] {
        let l = d.literal(test, name);
        d.executes(l, "test.clear");
    }
    let all = d.literal(test, "clearall");
    d.executes(all, "test.clear");
    let r = d.argument(all, "radius", int.clone());
    d.executes(r, "test.clear");
    let stop = d.literal(test, "stop");
    d.executes(stop, "test.stop");
    let pos = d.literal(test, "pos");
    d.executes(pos, "test.pos");
    let var = d.argument(pos, "var", ArgType::Word);
    d.executes(var, "test.pos");
    let create = d.literal(test, "create");
    let id = d.argument(create, "id", ArgType::Id("none"));
    d.executes(id, "test.create");
    let w = d.argument(id, "width", int.clone());
    d.executes(w, "test.create");
    let h = d.argument(w, "height", int.clone());
    let depth = d.argument(h, "depth", int);
    d.executes(depth, "test.create");
}

/// The commands only a dedicated server registers
/// (`Commands.CommandSelection.includeDedicated`): an integrated server, and
/// so single player and a LAN world, has none of them.
pub fn register_dedicated(d: &mut Dispatcher, root: usize) {
    // BanIpCommands, BanListCommands
    let ban_ip = d.literal(root, "ban-ip");
    d.requires(ban_ip, ADMINS);
    let t = d.argument(ban_ip, "target", ArgType::Word);
    d.executes(t, "ban-ip");
    let reason = d.argument(t, "reason", ArgType::Message);
    d.executes(reason, "ban-ip");
    let banlist = d.literal(root, "banlist");
    d.requires(banlist, ADMINS);
    d.executes(banlist, "banlist");
    for (word, id) in [("ips", "banlist.ips"), ("players", "banlist.players")] {
        let w = d.literal(banlist, word);
        d.executes(w, id);
    }

    // OpCommand, DeOpCommands, BanPlayerCommands, PardonCommand
    for (name, id, suggests) in [
        ("op", "op", Suggests::NonOperators),
        ("deop", "deop", Suggests::Operators),
        ("pardon", "pardon", Suggests::BannedPlayers),
    ] {
        let n = d.literal(root, name);
        d.requires(n, ADMINS);
        let t = d.argument(n, "targets", ArgType::GameProfile);
        d.suggests(t, suggests);
        d.executes(t, id);
    }
    // PardonIpCommand
    let pardon_ip = d.literal(root, "pardon-ip");
    d.requires(pardon_ip, ADMINS);
    let t = d.argument(pardon_ip, "target", ArgType::Word);
    d.suggests(t, Suggests::BannedIps);
    d.executes(t, "pardon-ip");

    let ban = d.literal(root, "ban");
    d.requires(ban, ADMINS);
    let t = d.argument(ban, "targets", ArgType::GameProfile);
    d.executes(t, "ban");
    let reason = d.argument(t, "reason", ArgType::Message);
    d.executes(reason, "ban");

    // WhitelistCommand
    let whitelist = d.literal(root, "whitelist");
    d.requires(whitelist, ADMINS);
    for (word, id) in [
        ("on", "whitelist.on"),
        ("off", "whitelist.off"),
        ("list", "whitelist.list"),
    ] {
        let w = d.literal(whitelist, word);
        d.executes(w, id);
    }
    for (word, id, suggests) in [
        ("add", "whitelist.add", Suggests::NonWhitelisted),
        ("remove", "whitelist.remove", Suggests::Whitelisted),
    ] {
        let w = d.literal(whitelist, word);
        let t = d.argument(w, "targets", ArgType::GameProfile);
        d.suggests(t, suggests);
        d.executes(t, id);
    }
    let reload = d.literal(whitelist, "reload");
    d.executes(reload, "whitelist.reload");
}
