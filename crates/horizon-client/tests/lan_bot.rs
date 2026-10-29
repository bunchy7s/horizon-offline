//! A protocol bot joins a hosted world the way a vanilla client does:
//! handshake, status, offline login, configuration (registries and tags),
//! then the play state (login, chunks with light, player list, entities,
//! block changes). Needs the prepared resources; skipped without them.
use horizon_client::{
    chunk::{Chunk, MIN_Y},
    lan::{
        Lan,
        chunks::ChunkEncoder,
        data::SyncData,
        entity::EntityState,
        hub::{HostEvent, HostState, Hub, HubConfig, TickCtx},
    },
    world::World,
};
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation},
        handshake, login,
        play::{self, Clientbound as C, Serverbound as S},
        status,
    },
};
use horizon_world::registry::BlockStateId;
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn resources() -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    root.join("items.json").exists().then_some(root)
}

const STONE: u32 = 1;

fn flat_world() -> World {
    let mut world = World::new(BlockStateId(0), 0);
    for cz in -4..=4 {
        for cx in -4..=4 {
            let mut chunk = Chunk::empty(cx, cz);
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, MIN_Y + 4, z, STONE);
                }
            }
            chunk.lit = true;
            world.chunks.insert((cx, cz), Arc::new(chunk));
        }
    }
    world
}

struct Bot {
    c: Connection,
    name: String,
}
impl Bot {
    fn recv_play(&mut self) -> Result<C, String> {
        let (id, payload) = self.c.recv().map_err(|e| e.to_string())?;
        match C::decode(id, &payload) {
            Ok(p) => Ok(p),
            // The command tree has argument nodes the client side does not decode.
            Err(e) if id == horizon_protocol::ids::play::clientbound::COMMANDS => {
                let _ = e;
                Ok(C::Unknown(id, payload))
            }
            Err(e) => Err(format!("packet {id}: {e}")),
        }
    }
    fn send(&mut self, p: S) {
        self.c.send(&p.encode()).unwrap();
    }
}

fn connect(port: u16, name: &str, known_pack: bool) -> Result<Bot, String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).map_err(e)?;
    c.reader
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(e)?;
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .map_err(e)?;
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: offline_uuid(name),
        })
        .encode(),
    )
    .map_err(e)?;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match login::Clientbound::decode(id, &payload).map_err(e)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(p) => {
                assert_eq!(p.profile.name, name);
                assert_eq!(p.profile.uuid, offline_uuid(name));
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(e)?;
                break;
            }
            login::Clientbound::Disconnect(d) => return Err(d.reason_json),
            other => return Err(format!("unexpected login packet {other:?}")),
        }
    }
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(4)).encode())
        .map_err(e)?;
    let (mut registries, mut tags, mut features) = (0, 0, 0);
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match cfg::Clientbound::decode(id, &payload).map_err(e)? {
            cfg::Clientbound::SelectKnownPacks(p) => {
                assert_eq!(p.packs.len(), 1);
                assert_eq!(p.packs[0].id, "core");
                let reply = if known_pack { p.packs } else { Vec::new() };
                c.send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: reply })
                        .encode(),
                )
                .map_err(e)?;
            }
            cfg::Clientbound::RegistryData(r) => {
                registries += 1;
                let has_data = r.entries.iter().any(|e| e.data.is_some());
                assert_eq!(has_data, !known_pack, "{}", r.registry);
            }
            cfg::Clientbound::UpdateTags(t) => tags += t.registries.len(),
            cfg::Clientbound::UpdateEnabledFeatures(_) => features += 1,
            cfg::Clientbound::FinishConfiguration(_) => {
                c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .map_err(e)?;
                break;
            }
            _ => {}
        }
    }
    assert!(registries >= 28, "registries: {registries}");
    assert!(tags >= 5, "tag registries: {tags}");
    assert_eq!(features, 1);
    Ok(Bot {
        c,
        name: name.into(),
    })
}

/// Drive the hub like the game loop does until `done` is true.
struct Server {
    hub: Hub,
    world: World,
    next_entity: u32,
    placed: Vec<(u32, [i32; 3], u32, i32)>,
}
impl Server {
    fn tick(&mut self, entities: &[EntityState]) {
        let host = HostState {
            uuid: 42,
            name: "Host".into(),
            entity_id: 1,
            position: [0.5, -59.0, 0.5],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: true,
            game_mode: 1,
            sneaking: false,
            sprinting: false,
            equipment: Default::default(),
            health: 20.0,
            attributes: Vec::new(),
            bed: None,
            living_flags: 0,
            body: Default::default(),
        };
        let mut ctx = TickCtx {
            world: &mut self.world,
            host: Some(&host),
            entities,
            next_entity: &mut self.next_entity,
            spawn: [8.5, f64::from(MIN_Y + 5), 8.5],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: (0, 0),
            collision: None,
            border: &horizon_client::worldborder::WorldBorder::default(),
        };
        let events = self.hub.tick(&mut ctx);
        for event in events {
            match event {
                HostEvent::Joined(conn) => self.hub.place_new_player(conn, &mut ctx),
                HostEvent::Packet(conn, packet) => {
                    if let S::UseItemOn(p) = *packet {
                        let at = [p.pos[0], p.pos[1] + 1, p.pos[2]];
                        ctx.world
                            .set(at[0], at[1], at[2], BlockStateId(STONE))
                            .unwrap();
                        self.placed.push((conn, at, STONE, p.sequence));
                        self.hub.ack(conn, p.sequence);
                    }
                }
                _ => {}
            }
        }
    }
}

fn server(resources: &Path) -> Server {
    let sync = Arc::new(SyncData::load(resources).expect("sync data"));
    let lan = Lan::start(sync.clone(), 0, "Test - World".into(), 8).unwrap();
    let biomes: Vec<String> = sync
        .dynamic
        .registries
        .get("minecraft:worldgen/biome")
        .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
        .unwrap_or_default();
    let states = vec![
        (Arc::<str>::from("minecraft:air"), false),
        (Arc::<str>::from("minecraft:stone"), false),
        (Arc::<str>::from("minecraft:chest"), false),
    ];
    let encoder = ChunkEncoder::new(states, &sync.builtin, &biomes, &biomes);
    let hub = Hub::new(
        lan,
        sync,
        encoder,
        HubConfig {
            default_game_mode: 1,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 3,
            simulation_distance: 3,
            dedicated: false,
            integrated: false,
            dimension: "minecraft:overworld".into(),
            seed: 1234,
            max_players: 8,
        },
    );
    Server {
        hub,
        world: flat_world(),
        next_entity: 100,
        placed: Vec::new(),
    }
}

/// Tick the server while a bot thread runs.
fn run<T: Send + 'static>(
    s: &mut Server,
    bot: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> T {
    let handle = thread::spawn(bot);
    let start = Instant::now();
    while !handle.is_finished() {
        s.tick(&[]);
        thread::sleep(Duration::from_millis(10));
        assert!(start.elapsed() < Duration::from_secs(40), "timed out");
    }
    handle.join().unwrap().unwrap()
}

#[test]
fn recipe_settings_packets_update_only_the_players_book_and_survive_save() {
    let Some(resources) = resources() else {
        return;
    };
    let mut s = server(&resources);
    let port = s.hub.lan.port;
    let _bot = run(&mut s, move || connect(port, "RecipeSettings", true));
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.hub.guests.is_empty() {
        s.tick(&[]);
        assert!(Instant::now() < deadline, "guest did not join");
        thread::yield_now();
    }
    for book in 0..4 {
        for flags in 0..4 {
            let open = flags & 1 != 0;
            let filtering = flags & 2 != 0;
            let before = s.hub.guests[0].recipe_book.clone();
            // Vanilla's wire payload: RecipeBookType VarInt, open, filtering.
            let packet = S::decode(
                horizon_protocol::ids::play::serverbound::RECIPE_BOOK_CHANGE_SETTINGS,
                &[book, u8::from(open), u8::from(filtering)],
            )
            .unwrap();
            s.hub.guests[0].inbox.push_back(packet);
            s.tick(&[]);
            let settings = &s.hub.guests[0].recipe_book;
            for i in 0..4 {
                assert_eq!(
                    settings.open[i],
                    if i == usize::from(book) {
                        open
                    } else {
                        before.open[i]
                    }
                );
                assert_eq!(
                    settings.filtering[i],
                    if i == usize::from(book) {
                        filtering
                    } else {
                        before.filtering[i]
                    }
                );
            }
            let mut saved = horizon_client::save::Level::default();
            horizon_client::lan::playerdata::store(&s.hub.guests[0], &mut saved);
            let mut reopened = horizon_client::save::Level::default();
            reopened.read_player(&saved.player_tag());
            assert_eq!(reopened.recipe_book.open, settings.open);
            assert_eq!(reopened.recipe_book.filtering, settings.filtering);
        }
    }
    for book_type in [-1, 4] {
        let mut wire = horizon_protocol::buf::Writer::new();
        wire.varint(book_type);
        wire.bool(true);
        wire.bool(true);
        assert!(
            S::decode(
                horizon_protocol::ids::play::serverbound::RECIPE_BOOK_CHANGE_SETTINGS,
                &wire.data,
            )
            .is_err()
        );
    }
}

#[test]
fn sprint_commands_survive_keyboard_input_changes() {
    let Some(resources) = resources() else {
        return;
    };
    let mut s = server(&resources);
    let port = s.hub.lan.port;
    let _bot = run(&mut s, move || connect(port, "Sprinter", true));
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.hub.guests.is_empty() {
        s.tick(&[]);
        assert!(Instant::now() < deadline, "guest did not join");
        thread::yield_now();
    }
    let id = s.hub.guests[0].entity_id;
    s.hub.guests[0].loaded = true;
    s.hub.guests[0]
        .inbox
        .push_back(S::PlayerCommand(play::PlayerCommand {
            id,
            action: 1,
            data: 0,
        }));
    s.tick(&[]);
    assert!(s.hub.guests[0].sprinting);
    // LocalPlayer keeps sprinting after Ctrl is released while W remains
    // down. Double-tap sprint also sends forward without the sprint key.
    for flags in [65, 1, 0, 33] {
        s.hub.guests[0]
            .inbox
            .push_back(S::PlayerInput(play::PlayerInput { flags }));
        s.tick(&[]);
        let g = &s.hub.guests[0];
        assert!(
            g.sprinting,
            "keyboard input {flags} cleared the sprint flag"
        );
        assert_eq!(g.input, flags);
        assert_eq!(g.sneaking, flags & 32 != 0);
    }
    s.hub.guests[0]
        .inbox
        .push_back(S::PlayerCommand(play::PlayerCommand {
            id,
            action: 2,
            data: 0,
        }));
    s.tick(&[]);
    assert!(!s.hub.guests[0].sprinting);
    s.hub.guests[0]
        .inbox
        .push_back(S::PlayerInput(play::PlayerInput { flags: 65 }));
    s.tick(&[]);
    assert!(
        !s.hub.guests[0].sprinting,
        "keyboard input started a sprint"
    );
}

#[test]
fn a_move_behind_a_dig_and_its_swing_waits_for_the_next_tick() {
    let Some(resources) = resources() else {
        return;
    };
    let mut s = server(&resources);
    let port = s.hub.lan.port;
    let _bot = run(&mut s, move || connect(port, "Digger", true));
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.hub.guests.is_empty() {
        s.tick(&[]);
        assert!(Instant::now() < deadline, "guest did not join");
        thread::yield_now();
    }
    s.hub.guests[0].loaded = true;
    let at = s.hub.guests[0].position;
    let below = [
        at[0].floor() as i32,
        at[1].floor() as i32 - 1,
        at[2].floor() as i32,
    ];
    // A creative break as the client sends it in one tick: the dig, the arm swing,
    // then the move into the block it predicted gone.
    let inbox = &mut s.hub.guests[0].inbox;
    inbox.push_back(S::PlayerAction(play::PlayerAction {
        action: 0,
        pos: below,
        direction: 1,
        sequence: 1,
    }));
    inbox.push_back(S::Punch(Empty));
    inbox.push_back(S::MovePlayerPos(play::MovePlayerPos {
        x: at[0],
        y: at[1] - 0.0784,
        z: at[2],
        flags: 0,
    }));
    s.tick(&[]);
    // The game breaks the block after the hub's pass: the move waits, or it would be
    // checked against the block and sent back.
    let g = &s.hub.guests[0];
    assert_eq!(g.position, at, "the move was handled before the dig");
    assert!(
        matches!(g.inbox.back(), Some(S::MovePlayerPos(_))),
        "the move left the queue"
    );
}

#[test]
fn status_and_join() {
    let Some(resources) = resources() else {
        return;
    };
    let mut s = server(&resources);
    let port = s.hub.lan.port;
    // Server list ping.
    let json = run(&mut s, move || {
        let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port,
                intent: handshake::STATUS,
            })
            .encode(),
        )
        .unwrap();
        c.send(&status::Serverbound::StatusRequest(Empty).encode())
            .unwrap();
        let (id, payload) = c.recv().map_err(|e| e.to_string())?;
        let status::Clientbound::StatusResponse(r) =
            status::Clientbound::decode(id, &payload).map_err(|e| e.to_string())?
        else {
            return Err("no status".into());
        };
        Ok(r.json)
    });
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        v["version"]["protocol"],
        horizon_core::version::PROTOCOL_VERSION
    );
    // ServerStatus.CODEC writes a plain literal MOTD as a bare string.
    assert_eq!(v["description"], "Test - World");

    // Join, receive the world, place a block, see another player arrive.
    let report = run(&mut s, move || {
        let mut bot = connect(port, "Steve", true)?;
        let mut chunks = 0;
        let (mut login_seen, mut teleported, mut time, mut info) = (false, false, false, false);
        let mut commands = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut sent_loaded = false;
        let mut block_update = None;
        while Instant::now() < deadline {
            match bot.recv_play()? {
                C::Login(l) => {
                    assert_eq!(l.chunk_radius, 3);
                    assert_eq!(l.spawn.dimension, "minecraft:overworld");
                    assert_eq!(l.spawn.game_type, 1);
                    login_seen = true;
                }
                C::PlayerPosition(p) => {
                    assert_eq!(p.position[0], 8.5);
                    bot.send(S::AcceptTeleportation(play::AcceptTeleportation {
                        id: p.teleport,
                        x: p.position[0],
                        y: p.position[1],
                        z: p.position[2],
                        yaw: 0.0,
                        pitch: 0.0,
                    }));
                    teleported = true;
                }
                // The join's full clock sync; MinecraftServer's 20-tick
                // forceGameTimeSynchronization sends no clocks and may come first.
                C::SetTime(t) if !t.clocks.is_empty() => time = true,
                C::PlayerInfoUpdate(u) => {
                    info |= u.entries.iter().any(|e| e.name.as_deref() == Some("Host"));
                }
                C::Commands(_) => commands = true,
                C::LevelChunkWithLight(l) => {
                    chunks += 1;
                    let sections = l.chunk.sections(24, 16, 7).map_err(|e| e.to_string())?;
                    // The floor is in the lowest section.
                    assert_eq!(sections[0].states.get(4 * 256), STONE as i32);
                    assert_eq!(sections[0].non_empty, 256);
                    assert_eq!(sections[1].non_empty, 0);
                    assert!(!l.light.sky.is_empty());
                }
                C::ChunkBatchFinished(b) => {
                    assert!(b.size > 0);
                    bot.send(S::ChunkBatchReceived(play::ChunkBatchReceived {
                        chunks_per_tick: 20.0,
                    }));
                    if !sent_loaded && chunks >= 9 {
                        bot.send(S::PlayerLoaded(Empty));
                        sent_loaded = true;
                        bot.send(S::MovePlayerPosRot(play::MovePlayerPosRot {
                            x: 9.5,
                            y: f64::from(MIN_Y + 5),
                            z: 9.5,
                            yaw: 90.0,
                            pitch: 10.0,
                            flags: 1,
                        }));
                        bot.send(S::UseItemOn(play::UseItemOn {
                            hand: 0,
                            pos: [9, MIN_Y + 4, 9],
                            face: 1,
                            cursor: [0.5, 1.0, 0.5],
                            inside: false,
                            world_border: false,
                            sequence: 77,
                        }));
                    }
                }
                C::BlockUpdate(u) => {
                    block_update = Some((u.pos, u.state));
                }
                C::BlockChangedAck(a) if a.sequence == 77 && block_update.is_some() => break,
                _ => {}
            }
        }
        assert!(
            login_seen && teleported && time && info && commands,
            "missing login data"
        );
        assert!(chunks >= 9, "only {chunks} chunks");
        assert_eq!(
            block_update,
            Some(([9, MIN_Y + 5, 9], STONE as i32)),
            "placed block was not broadcast"
        );
        Ok(bot.name)
    });
    assert_eq!(report, "Steve");
    assert_eq!(s.placed.len(), 1);
}

#[test]
fn guests_see_each_other_and_the_world_entities() {
    let Some(resources) = resources() else {
        return;
    };
    let mut s = server(&resources);
    let port = s.hub.lan.port;
    let zombie = EntityState {
        id: 500,
        uuid: 9,
        kind: "minecraft:zombie".into(),
        position: [12.0, f64::from(MIN_Y + 5), 12.0],
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground: true,
        velocity: [0.0; 3],
        data: 0,
        meta: Vec::new(),
        defaults: Vec::new(),
        equipment: Vec::new(),
        attributes: Vec::new(),
        precise: false,
        needs_sync: false,
        leash: None,
    };
    let first = thread::spawn(move || {
        let mut bot = connect(port, "Alex", false)?;
        bot.c
            .reader
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut zombies = 0;
        let mut players = 0;
        let mut moves = 0;
        let mut at: Option<[f64; 3]> = None;
        let mut reminder = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !(zombies > 0 && players > 0 && moves > 0) {
            // A vanilla client sends its position at least every 20 ticks even
            // when standing still (LocalPlayer's positionReminder), and the
            // server checks which entities it sees again on each one.
            if let Some([x, y, z]) = at
                && reminder.elapsed() >= Duration::from_secs(1)
            {
                bot.send(S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags: 1 }));
                reminder = Instant::now();
            }
            let Ok((id, payload)) = bot.c.recv() else {
                continue;
            };
            let Ok(packet) = C::decode(id, &payload) else {
                continue;
            };
            match packet {
                C::PlayerPosition(p) => {
                    at = Some(p.position);
                    bot.send(S::AcceptTeleportation(play::AcceptTeleportation {
                        id: p.teleport,
                        x: p.position[0],
                        y: p.position[1],
                        z: p.position[2],
                        yaw: 0.0,
                        pitch: 0.0,
                    }))
                }
                C::AddEntity(a) if a.id == 500 => zombies += 1,
                C::AddEntity(a) if a.uuid == offline_uuid("Bob") => players += 1,
                C::MoveEntityPos(_) | C::MoveEntityPosRot(_) => moves += 1,
                _ => {}
            }
        }
        if zombies > 0 && players > 0 && moves > 0 {
            Ok(())
        } else {
            Err(format!("zombies {zombies} players {players} moves {moves}"))
        }
    });
    let second = thread::spawn(move || {
        thread::sleep(Duration::from_millis(600));
        let mut bot = connect(port, "Bob", true)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut x = 8.5;
        while Instant::now() < deadline {
            // Keep reading and walking so the other player sees movement.
            bot.c
                .reader
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            if let Ok((id, payload)) = bot.c.recv()
                && let Ok(C::PlayerPosition(p)) = C::decode(id, &payload)
            {
                bot.send(S::AcceptTeleportation(play::AcceptTeleportation {
                    id: p.teleport,
                    x: p.position[0],
                    y: p.position[1],
                    z: p.position[2],
                    yaw: 0.0,
                    pitch: 0.0,
                }));
            }
            x += 0.2;
            bot.send(S::MovePlayerPos(play::MovePlayerPos {
                x,
                y: f64::from(MIN_Y + 5),
                z: 8.5,
                flags: 1,
            }));
        }
        Ok::<(), String>(())
    });
    let start = Instant::now();
    while !first.is_finished() {
        s.tick(std::slice::from_ref(&zombie));
        thread::sleep(Duration::from_millis(10));
        assert!(start.elapsed() < Duration::from_secs(40), "timed out");
    }
    first.join().unwrap().unwrap();
    drop(second);
}

/// `ServerGamePacketListenerImpl.handleContainerClick`: a spectator looks
/// into a chest but its clicks change nothing, and `sendAllDataToRemote`
/// takes back what its client predicted.
#[test]
fn a_spectators_container_click_is_taken_back() {
    use horizon_client::{containers::BlockEntity, items::ItemStack, lan::menu::Slots};
    struct NoRules;
    impl horizon_client::lan::menu::Rules for NoRules {
        fn items(&self) -> &horizon_client::items::Items {
            unreachable!("a spectator's click never reaches the menu")
        }
        fn max_stack(&self, _: &str) -> u32 {
            64
        }
        fn armor_index(&self, _: &str) -> Option<usize> {
            None
        }
    }
    let Some(resources) = resources() else {
        return;
    };
    let mut s = server(&resources);
    let port = s.hub.lan.port;
    let mut bot = run(&mut s, move || connect(port, "Watcher", true));
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.hub.guests.is_empty() {
        s.tick(&[]);
        assert!(Instant::now() < deadline, "guest did not join");
        thread::yield_now();
    }
    let conn = s.hub.guests[0].conn;
    s.hub.set_game_mode(conn, 3);
    let at = s.hub.guests[0].position;
    let key = (
        at[0].floor() as i32,
        at[1].floor() as i32,
        at[2].floor() as i32 + 2,
    );
    s.world.set(key.0, key.1, key.2, BlockStateId(2)).unwrap();
    let mut items = vec![None; 27];
    items[0] = Some(ItemStack::new("minecraft:diamond", 5));
    s.world
        .block_entities
        .insert(key, BlockEntity::Chest(items.clone()));
    s.hub.open_container(
        conn,
        "minecraft:generic_9x3",
        horizon_protocol::text::translatable("container.chest", &[]),
        key,
        Slots::Block(key),
        items.clone(),
    );
    let window = s.hub.guests[0].menu.as_ref().unwrap().window;
    // The menu's own contents, as the client first sees them.
    bot.c
        .reader
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let content = move |bot: &mut Bot| -> Result<(), String> {
        loop {
            if let C::ContainerSetContent(p) = bot.recv_play()?
                && p.container == window
            {
                assert!(p.items[0].is_some(), "the diamonds are still in slot 0");
                assert!(p.carried.is_none(), "nothing is carried");
                return Ok(());
            }
        }
    };
    let mut bot = run(&mut s, move || content(&mut bot).map(|()| bot));
    // The client predicted picking up the diamonds.
    let click = play::ContainerClick {
        container: window,
        state: 1,
        slot: 0,
        button: 0,
        input: 0,
        changed: vec![(0, None)],
        carried: None,
    };
    let drops = s.hub.container_click(conn, &click, &mut s.world, &NoRules);
    assert!(drops.is_empty());
    assert_eq!(
        s.world
            .block_entities
            .get(&key)
            .map(|b| b.slots()[0].clone()),
        Some(items[0].clone()),
        "the chest keeps its diamonds"
    );
    assert!(s.hub.guests[0].carried.is_none());
    run(&mut s, move || content(&mut bot));
}
