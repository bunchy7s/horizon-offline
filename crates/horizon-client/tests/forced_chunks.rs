//! A `/forceload`ed chunk reaches vanilla's rings (#2785): vanilla holds a
//! FORCED ticket at level 31 (`ChunkMap.FORCED_TICKET_LEVEL`), the level
//! rises one per ring (`ChunkTracker.computeLevelFromNeighbor`) and picks the
//! column's status (`ChunkLevel.generationStatus`). The chunk itself and the
//! two rings around it are level chunks with their light, the third ring has
//! its features and the fourth only its terrain.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::Chunk,
    level_loading::Status,
    light::LightTables,
    mesher::MeshTables,
    models,
    section_mask::SectionMask,
    streaming::{ChunkGenerator, Proto, Streaming, WorkerContext},
    world::World,
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

/// A column under generation, and whether it has its features.
struct Column {
    x: i32,
    z: i32,
    decorated: bool,
}

/// A generator that makes every column: a proto, decorated on request, and
/// finished into a level chunk.
struct Plain;

impl ChunkGenerator for Plain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), x: i32, z: i32) -> Proto {
        Box::new(Column {
            x,
            z,
            decorated: false,
        })
    }
    fn decorates(&self) -> bool {
        true
    }
    /// The centre gets its features, as vanilla's FEATURES step does; the
    /// neighbours are only written to.
    fn decorate(&self, _: &mut (), region: &mut [Proto; 9]) {
        region[4].downcast_mut::<Column>().unwrap().decorated = true;
    }
    fn is_decorated(&self, proto: &Proto) -> bool {
        proto.downcast_ref::<Column>().is_some_and(|c| c.decorated)
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let column = *proto.downcast::<Column>().unwrap();
        Chunk::empty(column.x, column.z)
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        let column = proto.downcast_ref::<Column>()?;
        Some(Chunk::empty(column.x, column.z))
    }
    fn non_air_sections(&self, proto: &Proto) -> Option<SectionMask> {
        proto.downcast_ref::<Column>()?;
        Some(SectionMask::single(4))
    }
}

#[test]
fn forced_chunks_reach_vanillas_rings() {
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
    let mut streaming = Streaming::new(Arc::new(Plain), context, 2);
    // The player stands far away: only the forced chunk's own rings are
    // generated.
    streaming.center = (0, 0);
    streaming.radius = 2;
    streaming.forced.insert((40, -40));
    let mut world = World::new(BlockStateId(0), 0);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut quiet = 0;
    while quiet < 20 {
        assert!(Instant::now() < deadline, "streaming never settled");
        let before = world.chunks.len();
        let _ = streaming.update(&mut world);
        thread::sleep(Duration::from_millis(5));
        let idle = !streaming.busy() && before == world.chunks.len();
        quiet = if idle { quiet + 1 } else { 0 };
    }
    // The chunk itself and the two rings around it are level chunks with
    // their light.
    for (dx, dz) in (-2..=2).flat_map(|dx| (-2..=2).map(move |dz| (dx, dz))) {
        let (x, z) = (40 + dx, -40 + dz);
        assert!(
            world.chunks.contains_key(&(x, z)),
            "({x}, {z}) is not a level chunk"
        );
        assert!(streaming.is_lit(x, z), "({x}, {z}) has no light");
    }
    // The third ring has its features, the fourth only its terrain.
    for (dx, dz) in (-4i32..=4).flat_map(|dx| (-4i32..=4).map(move |dz| (dx, dz))) {
        let (x, z) = (40 + dx, -40 + dz);
        let ring = dx.abs().max(dz.abs());
        let status = streaming.latest_status(&world, x, z);
        // A level chunk's light layers count a neighbour at FEATURES as
        // vanilla's do one at INITIALIZE_LIGHT (#4294); not one at TERRAIN.
        let proto = streaming.proto_sections(x, z);
        if ring == 3 {
            assert_eq!(status, Some(Status::Features), "({x}, {z})");
            assert_eq!(proto, Some(SectionMask::single(4)), "({x}, {z})");
        } else if ring == 4 {
            assert_eq!(status, Some(Status::Terrain), "({x}, {z})");
            assert_eq!(proto, None, "({x}, {z})");
        } else {
            assert_eq!(proto, None, "({x}, {z})");
        }
    }
}
