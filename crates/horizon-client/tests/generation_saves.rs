//! Streaming keeps generation consistent when the player leaves an area and
//! comes back, and across a restart (#249): every column is finished once,
//! after its own decoration and its eight neighbours', and chunks that
//! decoration wrote to are saved rather than generated again without those
//! writes. Bastions lost whole chunks of pieces this way.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::Chunk,
    light::LightTables,
    mesher::MeshTables,
    models,
    streaming::{ChunkGenerator, Proto, Streaming, WorkerContext},
    world::World,
};
use horizon_world::registry::BlockStateId;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn resources() -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    root.join("items.json").exists().then_some(root)
}

/// A proto chunk that records the decorations that wrote to it.
#[derive(Clone)]
struct Recorded {
    x: i32,
    z: i32,
    decorated: bool,
    writes: Vec<(i32, i32)>,
}
/// Generation with an in-memory save, like `Persistent`.
#[derive(Default)]
struct Recorder {
    protos: Mutex<HashMap<(i32, i32), Recorded>>,
    chunks: Mutex<HashMap<(i32, i32), Chunk>>,
    /// Each column as it was when it was finished, once per finish.
    finished: Mutex<HashMap<(i32, i32), Vec<Recorded>>>,
    /// The proto chunks a light job was given a view of, as they were.
    viewed: Mutex<Vec<Recorded>>,
}
impl Recorder {
    fn save_chunk(&self, chunk: &Chunk) {
        let mut saved = chunk.clone();
        saved.fresh = false;
        self.chunks
            .lock()
            .unwrap()
            .insert((chunk.x, chunk.z), saved);
    }
}
impl ChunkGenerator for Recorder {
    type Worker = ();
    fn worker(&self) {}
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.chunks.lock().unwrap().get(&(x, z)).cloned()
    }
    fn proto(&self, _: &mut (), x: i32, z: i32) -> Proto {
        let saved = self.protos.lock().unwrap().get(&(x, z)).cloned();
        Box::new(saved.unwrap_or(Recorded {
            x,
            z,
            decorated: false,
            writes: Vec::new(),
        }))
    }
    fn decorates(&self) -> bool {
        true
    }
    fn decorate(&self, _: &mut (), region: &mut [Proto; 9]) {
        let center = region[4].downcast_ref::<Recorded>().unwrap();
        let center = (center.x, center.z);
        for proto in region.iter_mut() {
            proto
                .downcast_mut::<Recorded>()
                .unwrap()
                .writes
                .push(center);
        }
        region[4].downcast_mut::<Recorded>().unwrap().decorated = true;
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let proto = proto.downcast::<Recorded>().unwrap();
        let mut chunk = Chunk::empty(proto.x, proto.z);
        chunk.fresh = true;
        self.finished
            .lock()
            .unwrap()
            .entry((proto.x, proto.z))
            .or_default()
            .push(*proto);
        chunk
    }
    fn is_decorated(&self, proto: &Proto) -> bool {
        proto.downcast_ref::<Recorded>().unwrap().decorated
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        let proto = proto.downcast_ref::<Recorded>().unwrap();
        self.viewed.lock().unwrap().push(proto.clone());
        Some(Chunk::empty(proto.x, proto.z))
    }
    fn save_protos(&self, protos: &[&Proto]) {
        let mut saved = self.protos.lock().unwrap();
        for proto in protos {
            let proto = proto.downcast_ref::<Recorded>().unwrap();
            saved.insert((proto.x, proto.z), proto.clone());
        }
    }
    fn save_finished(&self, chunk: Chunk) {
        self.save_chunk(&chunk);
    }
}

/// The game's part: columns saved when they unload and when the world saves.
struct Session {
    streaming: Streaming,
    world: World,
    generator: Arc<Recorder>,
}
impl Session {
    fn new(generator: Arc<Recorder>, context: Arc<WorkerContext>) -> Self {
        let mut streaming = Streaming::new(generator.clone(), context, 3);
        streaming.radius = 4;
        Self {
            streaming,
            world: World::new(BlockStateId(0), 0),
            generator,
        }
    }
    fn update(&mut self) {
        let (_, unloaded) = self.streaming.update(&mut self.world);
        for chunk in unloaded {
            if self.world.modified.remove(&(chunk.x, chunk.z)) {
                self.generator.save_chunk(&chunk);
            }
        }
    }
    /// Run until generation around `center` is done and nothing changes.
    fn settle_at(&mut self, center: (i32, i32)) {
        self.streaming.center = center;
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut quiet = 0;
        while quiet < 40 {
            assert!(Instant::now() < deadline, "generation never settled");
            let before = (self.streaming.generated_total, self.world.chunks.len());
            self.update();
            thread::sleep(Duration::from_millis(5));
            let idle = self.streaming.pending() == 0
                && before == (self.streaming.generated_total, self.world.chunks.len());
            quiet = if idle { quiet + 1 } else { 0 };
        }
    }
    /// The autosave: proto chunks are saved on a worker, nothing waits.
    fn autosave(&mut self) {
        self.streaming.save(&mut self.world, false);
    }
    fn save(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !self.streaming.poll_save(&mut self.world) {
            assert!(Instant::now() < deadline, "incremental save never finished");
            thread::sleep(Duration::from_millis(1));
        }
        for key in std::mem::take(&mut self.world.modified) {
            if let Some(chunk) = self.world.chunks.get(&key) {
                self.generator.save_chunk(chunk);
            }
        }
    }
}

#[test]
fn chunks_left_behind_keep_their_decoration() {
    let Some(root) = resources() else {
        return;
    };
    let assets = Arc::new(Assets::load(&root).unwrap());
    let render = Arc::new(assets.reload(&root).unwrap());
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = models::load_visibility(&root, assets.registry).unwrap();
    let context = Arc::new(WorkerContext {
        tables: Arc::new(MeshTables::new(&assets, &render, &facts, visibility).unwrap()),
        light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
        biomes: Arc::new(Biomes::load(&root).unwrap()),
        assets,
        render: render.clone(),
    });
    let generator = Arc::new(Recorder::default());
    let mut session = Session::new(generator.clone(), context.clone());
    // Generate an area, walk away from it in steps (its edge unloads a ring
    // at a time), teleport far, come back past where the edge was; then
    // save, restart and look again.
    session.settle_at((0, 0));
    for step in 1..=4 {
        session.settle_at((step * 3, 0));
        session.autosave();
    }
    session.settle_at((300, 300));
    session.autosave();
    session.settle_at((0, 0));
    session.settle_at((-4, 0));
    session.save();
    drop(session);
    let mut session = Session::new(generator.clone(), context);
    session.settle_at((6, 0));
    session.settle_at((0, 0));
    let finished = generator.finished.lock().unwrap();
    let mut checked = 0;
    for (&(x, z), finishes) in finished.iter() {
        assert_eq!(
            finishes.len(),
            1,
            "({x}, {z}) was generated again after it was finished"
        );
        let column = &finishes[0];
        assert!(column.decorated, "({x}, {z}) was finished undecorated");
        let mut writes = column.writes.clone();
        writes.sort_unstable();
        let neighbours: Vec<(i32, i32)> = (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| (x + dx, z + dz)))
            .collect();
        assert_eq!(
            writes, neighbours,
            "({x}, {z}) must hold each neighbour's decoration exactly once"
        );
        checked += 1;
    }
    for x in -4..=4 {
        for z in (-4..=4).filter(|z| x * x + z * z <= 16) {
            assert!(
                session.world.chunks.contains_key(&(x, z)),
                "({x}, {z}) did not load back"
            );
        }
    }
    assert!(checked > 200, "only {checked} columns finished");
}

/// The columns at the edge light against their decorated neighbours past
/// the area; each such proto chunk is turned into a view once while it stays
/// as it is, however many columns it neighbours or light jobs wait on it.
#[test]
fn proto_chunks_are_viewed_once_per_state() {
    let Some(root) = resources() else {
        return;
    };
    let assets = Arc::new(Assets::load(&root).unwrap());
    let render = Arc::new(assets.reload(&root).unwrap());
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = models::load_visibility(&root, assets.registry).unwrap();
    let context = Arc::new(WorkerContext {
        tables: Arc::new(MeshTables::new(&assets, &render, &facts, visibility).unwrap()),
        light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
        biomes: Arc::new(Biomes::load(&root).unwrap()),
        assets,
        render: render.clone(),
    });
    let generator = Arc::new(Recorder::default());
    let mut session = Session::new(generator.clone(), context);
    for step in 0..=4 {
        session.settle_at((step * 3, 0));
    }
    let viewed = generator.viewed.lock().unwrap();
    let mut states: Vec<((i32, i32), Vec<(i32, i32)>)> = viewed
        .iter()
        .map(|p| ((p.x, p.z), p.writes.clone()))
        .collect();
    let total = states.len();
    states.sort_unstable();
    states.dedup();
    assert!(total > 20, "only {total} proto chunks were viewed");
    assert_eq!(
        states.len(),
        total,
        "a proto chunk was viewed again unchanged"
    );
}

/// A saved proto chunk comes back exactly: the bastion region of #249, its
/// centre decorated (writing into all nine chunks), saved and restored onto
/// chunks generated again finishes into the same columns.
#[test]
fn saved_proto_chunks_restore_exactly() {
    use horizon_client::{
        save::{Persistent, SaveDir},
        vanillagen::VanillaTerrain,
    };
    let Some(root) = resources() else {
        return;
    };
    let assets = Arc::new(Assets::load(&root).unwrap());
    let biomes = Arc::new(Biomes::load(&root).unwrap());
    let terrain = VanillaTerrain::for_dimension(
        &root.join("worldgen"),
        assets.registry,
        &biomes,
        -1860902098141295193,
        "minecraft:the_nether",
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("horizon-proto-saves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = Arc::new(SaveDir::new(dir.clone()));
    let generator = Persistent {
        inner: Arc::new(terrain),
        save: save.clone(),
        registry: assets.clone(),
        biomes,
    };
    let mut worker = generator.worker();
    let keys: Vec<(i32, i32)> = (0..9).map(|i| (3 + i % 3 - 1, 7 + i / 3 - 1)).collect();
    let mut region: [Proto; 9] =
        std::array::from_fn(|i| generator.proto(&mut worker, keys[i].0, keys[i].1));
    generator.decorate(&mut worker, &mut region);
    assert!(generator.is_decorated(&region[4]));
    generator.save_protos(&region.iter().collect::<Vec<_>>());
    save.flush().unwrap();
    let restored: Vec<Proto> = keys
        .iter()
        .map(|&(x, z)| generator.proto(&mut worker, x, z))
        .collect();
    for (i, (original, restored)) in region.into_iter().zip(restored).enumerate() {
        assert_eq!(generator.is_decorated(&restored), i == 4);
        let (a, b) = (generator.finish(original), generator.finish(restored));
        let (x, z) = keys[i];
        for y in horizon_client::chunk::MIN_Y..horizon_client::chunk::MIN_Y + 384 {
            for lz in 0..16 {
                for lx in 0..16 {
                    assert_eq!(
                        a.get(lx, y, lz),
                        b.get(lx, y, lz),
                        "block {} {y} {} differs",
                        x * 16 + lx,
                        z * 16 + lz
                    );
                }
            }
        }
        assert_eq!(
            format!("{:?}", a.block_entities),
            format!("{:?}", b.block_entities)
        );
        assert_eq!(a.postprocess, b.postprocess, "chunk {x} {z}");
        assert_eq!(a.entities, b.entities, "chunk {x} {z}");
        assert_eq!(a.structure_refs, b.structure_refs, "chunk {x} {z}");
        let ticks = |c: &Chunk| {
            c.saved
                .as_ref()
                .map(|s| (s.block_ticks.clone(), s.fluid_ticks.clone()))
        };
        assert_eq!(ticks(&a), ticks(&b), "chunk {x} {z}");
    }
    drop(generator);
    drop(save);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saved_proto_block_entities_keep_their_id() {
    use horizon_client::{
        save::{Persistent, SaveDir},
        vanillagen::VanillaTerrain,
    };
    use horizon_world::{
        nbt::{Limits, Tag},
        region::RegionFile,
    };
    let Some(root) = resources() else {
        return;
    };
    let assets = Arc::new(Assets::load(&root).unwrap());
    let biomes = Arc::new(Biomes::load(&root).unwrap());
    let terrain = VanillaTerrain::for_dimension(
        &root.join("worldgen"),
        assets.registry,
        &biomes,
        12345,
        "minecraft:overworld",
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("horizon-proto-ids-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = Arc::new(SaveDir::new(dir.clone()));
    let generator = Persistent {
        inner: Arc::new(terrain),
        save: save.clone(),
        registry: assets.clone(),
        biomes,
    };
    let mut worker = generator.worker();
    // A cold ocean ruin of seed 12345: its suspicious gravel is placed with a
    // loot table and no type.
    let keys: Vec<(i32, i32)> = (0..9).map(|i| (43 + i % 3 - 1, 21 + i / 3 - 1)).collect();
    let mut region: [Proto; 9] =
        std::array::from_fn(|i| generator.proto(&mut worker, keys[i].0, keys[i].1));
    generator.decorate(&mut worker, &mut region);
    generator.save_protos(&region.iter().collect::<Vec<_>>());
    save.flush().unwrap();
    let mut file = RegionFile::open(dir.join("region/r.1.0.mca")).unwrap();
    let mut suspicious = 0;
    for &(x, z) in &keys {
        let Some(record) = file
            .read(horizon_core::position::ChunkPos::new(x, z))
            .unwrap()
        else {
            continue;
        };
        let root = record.nbt(Limits::default()).unwrap();
        let Some(Tag::List(entities)) = root.tag.get("block_entities") else {
            continue;
        };
        for e in entities {
            assert!(e.get("id").is_some(), "chunk {x} {z}: {e:?}");
            if let Some(Tag::String(table)) = e.get("LootTable") {
                suspicious += usize::from(table.to_string_lossy().contains("archaeology/"));
            }
        }
    }
    assert!(suspicious > 0, "the ruin placed no suspicious blocks");
    drop(generator);
    drop(save);
    let _ = std::fs::remove_dir_all(&dir);
}
