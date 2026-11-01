use super::*;
use crate::nbt::NamedTag;

struct TempWorld(PathBuf);
impl TempWorld {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("horizon-wu-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn folder(&self, dimension: &str, folder: &str) -> PathBuf {
        dimension_path(&self.0, dimension).join(folder)
    }
}
impl Drop for TempWorld {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root(tag: Tag) -> NamedTag {
    NamedTag {
        name: "".into(),
        tag,
    }
}

fn write_chunks(file: &Path, chunks: &[(ChunkPos, Tag)]) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut region = RegionFile::open_writable(file, true).unwrap();
    for (pos, tag) in chunks {
        let record =
            ChunkRecord::from_nbt(&root(tag.clone()), Compression::Zlib, 1, Limits::default())
                .unwrap();
        region.write(*pos, &record).unwrap();
    }
}

fn read_chunk(file: &Path, pos: ChunkPos) -> Option<Tag> {
    let mut region = RegionFile::open(file).unwrap();
    region
        .read(pos)
        .unwrap()
        .map(|r| r.nbt(Limits::default()).unwrap().tag)
}

fn entity_chunk(version: i32, x: i32, z: i32) -> Tag {
    Tag::compound([
        ("DataVersion", Tag::Int(version)),
        ("Position", Tag::IntArray(vec![x, z])),
        ("Entities", Tag::List(vec![])),
    ])
}

fn level(world: &TempWorld) -> Vec<String> {
    let _ = world;
    vec![OVERWORLD.into(), NETHER.into()]
}

fn run(world: &TempWorld, erase: bool, recreate: bool) -> Arc<UpgradeProgress> {
    let progress = Arc::new(UpgradeProgress::new());
    work(&world.0, &level(world), &progress, erase, recreate);
    progress
}

fn version_of(file: &Path, pos: ChunkPos) -> i32 {
    as_int(read_chunk(file, pos).unwrap().get("DataVersion"), -1)
}

#[test]
fn old_chunks_are_converted_and_current_ones_skipped() {
    let world = TempWorld::new("convert");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(4000, 0, 0)),
            (ChunkPos::new(0, 1), entity_chunk(CURRENT, 0, 1)),
        ],
    );
    let progress = run(&world, false, false);
    assert!(progress.is_finished());
    assert_eq!(version_of(&file, ChunkPos::new(0, 0)), CURRENT);
    assert_eq!(version_of(&file, ChunkPos::new(0, 1)), CURRENT);
    // The last data type run is the blocks; entities counted 1 converted, 1 skipped.
    assert_eq!(progress.data_fix_type(), Some(UpgradeDataFixType::Chunk));
    assert_eq!(progress.total_chunks(), 0);
}

#[test]
fn counts_of_one_data_type() {
    let world = TempWorld::new("counts");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(4000, 0, 0)),
            (ChunkPos::new(0, 1), entity_chunk(CURRENT, 0, 1)),
        ],
    );
    let progress = Arc::new(UpgradeProgress::new());
    upgrade_levels(
        &world.0,
        &level(&world),
        &progress,
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", false),
        |_, _| {},
    );
    assert_eq!(progress.total_chunks(), 2);
    assert_eq!(progress.converted(), 1);
    assert_eq!(progress.skipped(), 1);
    assert_eq!(progress.status(), UpgradeStatus::Finished);
    assert_eq!(progress.total_progress(), 1.0);
    assert_eq!(progress.dimension_progress(OVERWORLD), 1.0);
    assert_eq!(progress.dimension_progress(NETHER), 0.0);
}

/// `converted = converted && processOnePosition(..)`: after a chunk that was
/// not converted, the rest of its file is left alone.
#[test]
fn a_skipped_chunk_ends_the_files_pass() {
    let world = TempWorld::new("shortcircuit");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(CURRENT, 0, 0)),
            (ChunkPos::new(1, 0), entity_chunk(4000, 1, 0)),
        ],
    );
    let progress = Arc::new(UpgradeProgress::new());
    upgrade_levels(
        &world.0,
        &level(&world),
        &progress,
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", false),
        |_, _| {},
    );
    assert_eq!(progress.total_chunks(), 2);
    assert_eq!(progress.converted(), 0);
    assert_eq!(progress.skipped(), 1);
    assert_eq!(version_of(&file, ChunkPos::new(1, 0)), 4000);
}

#[test]
fn poi_chunks_without_a_version_start_at_1945() {
    let world = TempWorld::new("poi");
    let file = world.folder(NETHER, "poi").join("r.-1.2.mca");
    let pos = ChunkPos::new(-32, 64);
    write_chunks(
        &file,
        &[(
            pos,
            Tag::compound([("Sections", Tag::Compound(Default::default()))]),
        )],
    );
    run(&world, false, false);
    assert_eq!(version_of(&file, pos), CURRENT);
}

#[test]
fn erase_cache_drops_heightmaps_and_light() {
    let world = TempWorld::new("erase");
    let file = world.folder(OVERWORLD, "region").join("r.0.0.mca");
    let pos = ChunkPos::new(0, 0);
    let chunk = |version| {
        Tag::compound([
            ("DataVersion", Tag::Int(version)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            ("Status", Tag::String("minecraft:full".into())),
            ("isLightOn", Tag::Byte(1)),
            ("Heightmaps", Tag::Compound(Default::default())),
            (
                "sections",
                Tag::List(vec![Tag::compound([
                    ("Y", Tag::Byte(0)),
                    ("SkyLight", Tag::ByteArray(vec![0; 2048])),
                    ("BlockLight", Tag::ByteArray(vec![0; 2048])),
                ])]),
            ),
        ])
    };
    write_chunks(&file, &[(pos, chunk(CURRENT))]);
    // Without the option a current chunk is left as it is.
    let progress = run(&world, false, false);
    assert!(read_chunk(&file, pos).unwrap().get("Heightmaps").is_some());
    assert_eq!(progress.status(), UpgradeStatus::Finished);

    run(&world, true, false);
    let after = read_chunk(&file, pos).unwrap();
    assert!(after.get("Heightmaps").is_none());
    assert!(after.get("isLightOn").is_none());
    let Some(Tag::List(sections)) = after.get("sections") else {
        panic!("sections");
    };
    assert!(sections[0].get("SkyLight").is_none() && sections[0].get("BlockLight").is_none());
    assert!(sections[0].get("Y").is_some());
}

#[test]
fn erase_cache_modifier_reports_changes_only_when_something_went() {
    let pos = ChunkPos::new(3, 4);
    let mut bare = Tag::compound([("xPos", Tag::Int(3)), ("zPos", Tag::Int(4))]);
    assert!(!verify_chunk_pos_and_erase_cache(pos, &mut bare));
    let mut lit = Tag::compound([("xPos", Tag::Int(3)), ("isLightOn", Tag::Byte(1))]);
    assert!(verify_chunk_pos_and_erase_cache(pos, &mut lit));
    assert!(!verify_chunk_pos(pos, &mut lit));
}

#[test]
fn recreating_rewrites_every_file_and_removes_the_new_folder() {
    let world = TempWorld::new("recreate");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(CURRENT, 0, 0)),
            (ChunkPos::new(1, 0), entity_chunk(CURRENT, 1, 0)),
        ],
    );
    let before = fs::read(&file).unwrap();
    let progress = run(&world, false, true);
    assert!(progress.is_finished());
    assert!(!world.folder(OVERWORLD, "new_entities").exists());
    assert_eq!(version_of(&file, ChunkPos::new(1, 0)), CURRENT);
    assert_ne!(
        fs::read(&file).unwrap(),
        before,
        "the file was written anew"
    );
}

#[test]
fn cancel_stops_before_the_next_file() {
    let world = TempWorld::new("cancel");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(&file, &[(ChunkPos::new(0, 0), entity_chunk(4000, 0, 0))]);
    let progress = Arc::new(UpgradeProgress::new());
    let mut upgrader = RegionStorageUpgrader::init(
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", false),
        progress.clone(),
        0,
        OVERWORLD,
        &world.0,
    );
    progress.set_canceled();
    upgrader.upgrade();
    assert_eq!(progress.converted(), 0);
    assert_eq!(version_of(&file, ChunkPos::new(0, 0)), 4000);
}

#[test]
fn the_worker_thread_finishes_and_a_cancel_joins_it() {
    let world = TempWorld::new("thread");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(&file, &[(ChunkPos::new(0, 0), entity_chunk(4000, 0, 0))]);
    let mut upgrader = WorldUpgrader::new(&world.0, world_dimensions(&world.0), false, false);
    while !upgrader.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    upgrader.cancel();
    assert_eq!(version_of(&file, ChunkPos::new(0, 0)), CURRENT);
    assert_eq!(upgrader.levels(), [OVERWORLD, NETHER, END]);
}

#[test]
fn status_keys_follow_vanilla() {
    let progress = UpgradeProgress::new();
    assert_eq!(status_key(&progress), "optimizeWorld.stage.counting");
    progress.reset(UpgradeDataFixType::PoiChunk);
    progress.set_status(UpgradeStatus::Upgrading);
    assert_eq!(status_key(&progress), "optimizeWorld.stage.upgrading.poi");
    progress.set_status(UpgradeStatus::Finished);
    assert_eq!(status_key(&progress), "optimizeWorld.stage.finished.poi");
    progress.set_status(UpgradeStatus::Failed);
    assert_eq!(status_key(&progress), "optimizeWorld.stage.failed");
}

#[test]
fn region_names_and_dimension_folders() {
    assert_eq!(region_coordinates("r.-3.12.mca"), Some((-3, 12)));
    assert_eq!(region_coordinates("r.1.mca"), None);
    assert_eq!(region_coordinates("r.a.1.mca"), None);
    assert_eq!(region_coordinates("r.1.1.mcc"), None);
    let base = Path::new("w");
    assert_eq!(
        dimension_path(base, "mod:sky"),
        Path::new("w/dimensions/mod/sky")
    );
    let world = TempWorld::new("dims");
    fs::create_dir_all(world.0.join("dimensions/mod/sky")).unwrap();
    assert_eq!(
        world_dimensions(&world.0),
        [OVERWORLD, NETHER, END, "mod:sky"]
    );
}
