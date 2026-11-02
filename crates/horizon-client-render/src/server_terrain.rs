//! Far terrain on servers: every chunk a server sends is kept as its
//! far-terrain summary (`lod::Column`s), on disk per server, world and
//! dimension, and drawn beyond the server's view distance on this visit and
//! the next ones. A server never sends its seed, so its
//! distant terrain can't be generated as a local world's is.
//!
//! The summaries live in Anvil region files (`r.X.Z.mca`, one NBT record a
//! chunk), under `server-terrain/<server>/<hashed seed>/<namespace>/<level>/`.
//! Past what it has, the server's BlueMap may fill in (`bluemap`).
use crate::{
    bluemap::{BlueMap, Found},
    fast_hash::FxHashMap,
    lod::{Column, Span, Spans, Surface},
    streaming::{LodDetail, LodSampler, LodSource},
};
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    compression::Compression,
    nbt::{Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile},
    registry::BlockStateId,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Bumped when the summary layout changes; older records are ignored.
const FORMAT: i32 = 1;
/// Decoded chunks kept in memory, and open region files.
const MAX_CHUNKS: usize = 8192;
const MAX_REGIONS: usize = 64;
/// The widest cell, in chunks, drawn from the chunks kept under it
/// (`Sampler::reduced_column`): its regions all stay open.
const MAX_KEPT_SIDE: i32 = 128;

type Columns = Arc<[Column; 256]>;

/// The folder of a server's terrain: its address as typed, lowercased, with
/// the default port left out, and every character a file name can't hold
/// replaced (`play.example.net:25566` → `play.example.net_25566`).
pub fn server_folder(address: &str) -> String {
    let address = address.trim().to_ascii_lowercase();
    let address = address.strip_suffix(":25565").unwrap_or(&address);
    let name: String = address
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    if name.is_empty() {
        "_".into()
    } else {
        name.into()
    }
}

/// `root/<server>/<seed>/<namespace>/<path>` for a level key
/// (`minecraft:overworld`) and the login's hashed seed, which keeps
/// the worlds behind one address (a proxy's servers, a reset) apart.
pub fn level_dir(root: &Path, address: &str, hashed_seed: i64, level: &str) -> PathBuf {
    root.join(server_folder(address))
        .join(hashed_seed.to_string())
        .join(level_path(level))
}

/// `<namespace>/<path>` of a level key.
fn level_path(level: &str) -> PathBuf {
    let (namespace, path) = level.split_once(':').unwrap_or(("minecraft", level));
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('.')
            .to_owned()
    };
    let mut dir = PathBuf::from(clean(namespace));
    for part in path.split('/').map(clean).filter(|p| !p.is_empty()) {
        dir.push(part);
    }
    dir
}

struct Region {
    file: RegionFile,
    present: [bool; 1024],
}

/// Open region files by region; `None`: the file doesn't exist.
type Regions = FxHashMap<(i32, i32), Option<Region>>;

#[derive(Default)]
struct State {
    regions: Regions,
    /// `None`: the chunk isn't stored.
    chunks: FxHashMap<(i32, i32), Option<Columns>>,
}

/// One level's stored terrain, shared by the far-terrain threads. A chunk
/// it doesn't have comes from the server's BlueMap, if the player named it
/// or Find BlueMap found it.
pub struct ServerTerrain {
    dir: PathBuf,
    blue_map: Option<Arc<BlueMap>>,
    state: Mutex<State>,
    warned: std::sync::atomic::AtomicBool,
}

impl ServerTerrain {
    pub fn new(dir: PathBuf, blue_map: Option<Arc<BlueMap>>) -> Arc<Self> {
        Arc::new(Self {
            dir,
            blue_map,
            state: Mutex::default(),
            warned: Default::default(),
        })
    }

    /// BlueMap's coarser levels at block (x, z) (a void there is `Ready`).
    fn coarser(&self, x: i32, z: i32) -> Found<Column> {
        self.blue_map
            .as_ref()
            .map_or(Found::Absent, |map| map.coarser_column(x, z))
    }

    /// A chunk's columns: stored, else BlueMap's (`Waiting`: on its way).
    fn chunk_or_map(&self, x: i32, z: i32) -> Found<Columns> {
        if let Some(columns) = self.chunk(x, z) {
            return Found::Ready(columns);
        }
        self.blue_map
            .as_ref()
            .map_or(Found::Absent, |map| map.chunk(x, z))
    }

    /// The stored chunk nearest the middle of the `side` × `side` chunks
    /// from `(x, z)`, found in the region headers without reading any.
    #[cfg(test)]
    fn nearest(&self, x: i32, z: i32, side: i32) -> Option<(i32, i32)> {
        self.kept(x, z, side).1
    }

    /// How many of the `side` × `side` chunks from `(x, z)` are stored, and
    /// the one nearest their middle (`nearest`).
    fn kept(&self, x: i32, z: i32, side: i32) -> (usize, Option<(i32, i32)>) {
        let mut state = self.state.lock().unwrap();
        let middle = (x * 2 + side, z * 2 + side);
        let mut count = 0;
        let mut best: Option<(i64, (i32, i32))> = None;
        for rz in z >> 5..=(z + side - 1) >> 5 {
            for rx in x >> 5..=(x + side - 1) >> 5 {
                let present = region(&mut state.regions, &self.dir, (rx, rz))
                    .map_or([false; 1024], |r| r.present);
                for (i, _) in present.iter().enumerate().filter(|(_, p)| **p) {
                    let (cx, cz) = (rx * 32 + (i % 32) as i32, rz * 32 + (i / 32) as i32);
                    if cx < x || cz < z || cx >= x + side || cz >= z + side {
                        continue;
                    }
                    count += 1;
                    let (dx, dz) = (
                        i64::from(cx * 2 + 1 - middle.0),
                        i64::from(cz * 2 + 1 - middle.1),
                    );
                    let distance = dx * dx + dz * dz;
                    if best.is_none_or(|(d, _)| distance < d) {
                        best = Some((distance, (cx, cz)));
                    }
                }
            }
        }
        (count, best.map(|(_, chunk)| chunk))
    }

    /// A stored chunk's columns.
    pub fn chunk(&self, x: i32, z: i32) -> Option<Columns> {
        let key = (x >> 5, z >> 5);
        let here = {
            let mut state = self.state.lock().unwrap();
            if let Some(known) = state.chunks.get(&(x, z)) {
                return known.clone();
            }
            read(region(&mut state.regions, &self.dir, key), x, z)
        };
        // A record of another game version or layout is none.
        let columns = here.and_then(|record| {
            record
                .nbt(Limits::default())
                .ok()
                .and_then(|root| decode(&root.tag))
        });
        let mut state = self.state.lock().unwrap();
        // Stored by another thread meanwhile (`write`): that, newer than
        // what was read here.
        if let Some(Some(stored)) = state.chunks.get(&(x, z)) {
            return Some(stored.clone());
        }
        remember(&mut state, (x, z), columns.clone());
        columns
    }

    /// A chunk couldn't be kept (disk full, a folder it can't write): said
    /// once a level, not for every chunk after.
    fn warn_once(&self, error: &std::io::Error) {
        if !self.warned.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::warn!("Horizon: far terrain can't keep the server's chunks: {error}");
        }
    }

    /// Keep a chunk's columns, unless they are what is stored already.
    pub fn store(&self, key: (i32, i32), columns: &Columns) -> std::io::Result<()> {
        if self
            .chunk(key.0, key.1)
            .is_some_and(|old| *old == **columns)
        {
            return Ok(());
        }
        self.write(key, columns)
    }

    fn write(&self, (x, z): (i32, i32), columns: &Columns) -> std::io::Result<()> {
        let root = NamedTag {
            name: "".into(),
            tag: encode(x, z, columns),
        };
        let record = ChunkRecord::from_nbt(&root, Compression::Zlib, now(), Limits::default())?;
        let mut state = self.state.lock().unwrap();
        let key = (x >> 5, z >> 5);
        if region(&mut state.regions, &self.dir, key).is_none() {
            std::fs::create_dir_all(&self.dir)?;
            let file = RegionFile::open_writable(region_path(&self.dir, key), true)?;
            state.regions.insert(key, Some(Region::new(file)));
        }
        let region = region(&mut state.regions, &self.dir, key).expect("region just opened");
        region.file.write(ChunkPos::new(x, z), &record)?;
        region.present[slot(x, z)] = true;
        remember(&mut state, (x, z), Some(columns.clone()));
        Ok(())
    }
}

fn region_path(dir: &Path, (rx, rz): (i32, i32)) -> PathBuf {
    dir.join(format!("r.{rx}.{rz}.mca"))
}

/// A region of `dir`, opened on first use.
fn region<'a>(regions: &'a mut Regions, dir: &Path, key: (i32, i32)) -> Option<&'a mut Region> {
    if !regions.contains_key(&key) {
        if regions.len() >= MAX_REGIONS {
            regions.clear();
        }
        let file = RegionFile::open_writable(region_path(dir, key), false);
        regions.insert(key, file.ok().map(Region::new));
    }
    regions.get_mut(&key)?.as_mut()
}

fn read(region: Option<&mut Region>, x: i32, z: i32) -> Option<ChunkRecord> {
    let region = region.filter(|r| r.present[slot(x, z)])?;
    region.file.read(ChunkPos::new(x, z)).ok().flatten()
}

impl Region {
    fn new(file: RegionFile) -> Self {
        let mut present = [false; 1024];
        for (pos, _) in file.chunks() {
            present[slot(pos.x, pos.z)] = true;
        }
        Self { file, present }
    }
}

fn slot(x: i32, z: i32) -> usize {
    (x & 31) as usize + (z & 31) as usize * 32
}

fn remember(state: &mut State, key: (i32, i32), columns: Option<Columns>) {
    if state.chunks.len() >= MAX_CHUNKS {
        state.chunks.clear();
    }
    state.chunks.insert(key, columns);
}

fn now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

/// The columns as one int array: per column a surface (`0`, or `1`, height,
/// state, water flag, water height, water state, biome), the span count, and
/// three ints a span (min and max, state, biome and light).
fn encode(x: i32, z: i32, columns: &[Column; 256]) -> Tag {
    let mut ints = Vec::with_capacity(256 * 8);
    for column in columns {
        match column.surface {
            None => ints.push(0),
            Some(s) => {
                let (water, water_height, water_state) = s
                    .water
                    .map_or((0, 0, 0), |(h, state)| (1, h, state.0 as i32));
                ints.extend([
                    1,
                    s.height,
                    s.state.0 as i32,
                    water,
                    water_height,
                    water_state,
                    i32::from(s.biome),
                ]);
            }
        }
        ints.push(column.spans.len() as i32);
        for span in &column.spans {
            ints.extend([
                i32::from(span.min as u16) << 16 | i32::from(span.max as u16),
                span.state.0 as i32,
                i32::from(span.biome) << 16 | i32::from(span.light),
            ]);
        }
    }
    Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("HorizonFarTerrain", Tag::Int(FORMAT)),
        ("xPos", Tag::Int(x)),
        ("zPos", Tag::Int(z)),
        ("columns", Tag::IntArray(ints)),
    ])
}

/// `encode`'s columns, or `None` for a record of another game version or
/// layout (their block states and biomes are numbered differently).
fn decode(tag: &Tag) -> Option<Columns> {
    if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION))
        || tag.get("HorizonFarTerrain") != Some(&Tag::Int(FORMAT))
    {
        return None;
    }
    let Some(Tag::IntArray(ints)) = tag.get("columns") else {
        return None;
    };
    let mut ints = ints.iter().copied();
    let mut next = || ints.next();
    let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
    for column in &mut columns {
        column.surface = match next()? {
            0 => None,
            1 => {
                let height = next()?;
                let state = BlockStateId(next()? as u32);
                let water = next()?;
                let water_height = next()?;
                let water_state = BlockStateId(next()? as u32);
                let biome = next()? as u16;
                Some(Surface {
                    height,
                    state,
                    water: (water != 0).then_some((water_height, water_state)),
                    biome,
                })
            }
            _ => return None,
        };
        let count = usize::try_from(next()?).ok().filter(|&n| n <= 4096)?;
        column.spans = Spans::with_capacity(count);
        for _ in 0..count {
            let range = next()?;
            let state = BlockStateId(next()? as u32);
            let material = next()?;
            column.spans.push(Span {
                min: (range >> 16) as i16,
                max: range as i16,
                state,
                biome: (material >> 16) as u16,
                light: material as u16,
            });
        }
    }
    next().is_none().then(|| Arc::new(columns))
}

impl LodSource for ServerTerrain {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        Box::new(Sampler {
            terrain: self,
            details: Vec::new(),
            waited: false,
        })
    }
    fn keeps_observed(&self) -> bool {
        true
    }
}

impl Sampler {
    /// For a column whose chunk of the map is on its way: its coarser
    /// levels' meanwhile, as they come first (a void there is drawn as
    /// one). None there either is a hole (`waited`: the sample isn't drawn);
    /// either way the column is sampled again when its tile arrives
    /// (`Lod::refresh`).
    fn coarser(&mut self, x: i32, z: i32) -> Column {
        match self.terrain.coarser(x, z) {
            Found::Ready(column) => column,
            Found::Waiting | Found::Absent => {
                self.waited = true;
                Column::default()
            }
        }
    }
}

struct Sampler {
    terrain: Arc<ServerTerrain>,
    details: Vec<LodDetail>,
    /// A column was left empty for BlueMap's on its way, with no coarser
    /// level to stand in (`waited`).
    waited: bool,
}

impl LodSampler for Sampler {
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        Some(Box::new(Self {
            terrain: self.terrain.clone(),
            details: Vec::new(),
            waited: false,
        }))
    }
    /// Stored chunks come back whole, else BlueMap's (its coarser levels'
    /// where the finest is out of reach); the others are known empty. One
    /// whose BlueMap tile is still downloading is asked again later.
    fn request_detail(&mut self, chunks: &[(i32, i32)], _center: [i32; 2]) {
        for &(x, z) in chunks {
            let columns = match self.terrain.chunk_or_map(x, z) {
                Found::Ready(columns) => columns,
                Found::Waiting => continue,
                Found::Absent => Arc::new(std::array::from_fn(|_| Column::default())),
            };
            self.details.push(((x, z), columns));
        }
    }
    fn drain_details(&mut self) -> Vec<LodDetail> {
        std::mem::take(&mut self.details)
    }
    fn observed(&mut self, key: (i32, i32), columns: &Columns) {
        if let Err(e) = self.terrain.store(key, columns) {
            self.terrain.warn_once(&e);
        }
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> Column {
        match self.terrain.chunk_or_map(x >> 4, z >> 4) {
            Found::Ready(c) => c[((z & 15) * 16 + (x & 15)) as usize].clone(),
            Found::Waiting => self.coarser(x, z),
            Found::Absent => Column::default(),
        }
    }
    fn waited(&mut self) -> bool {
        std::mem::take(&mut self.waited)
    }
    /// A cell of `width` blocks from one stored chunk: its own area up to a
    /// chunk, else the stored chunk nearest the cell's middle (the server
    /// sent only the chunks a player came near), when most of the cell is
    /// stored. With a BlueMap, a wider
    /// cell takes the chunk at its middle if stored, else the map's pixel
    /// there, and the nearest stored chunk only while the map has none.
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> Column {
        if width <= 16 {
            let chunk = match self.terrain.chunk_or_map(x >> 4, z >> 4) {
                Found::Ready(chunk) => chunk,
                Found::Waiting => return self.coarser(x, z),
                Found::Absent => return Column::default(),
            };
            let width = width.max(1) as usize;
            let lx = (x & 15) as usize & !(width - 1);
            let lz = (z & 15) as usize & !(width - 1);
            return Column::reduced(&chunk, lx, lz, width);
        }
        let mut waiting = false;
        if let Some(map) = &self.terrain.blue_map {
            let (mx, mz) = ((x + width / 2) >> 4, (z + width / 2) >> 4);
            if let Some(chunk) = self.terrain.chunk(mx, mz) {
                return Column::reduced(&chunk, 0, 0, 16);
            }
            match map.reduced_column(x, z, width) {
                Found::Ready(column) => return column,
                Found::Waiting => waiting = true,
                Found::Absent => {}
            }
        }
        // Only over a cell the server sent most of, as `Column::reduced`
        // keeps what fills half a cell: one chunk drawn as a whole cell stood
        // as a flat slab far wider than anything seen there, until finer
        // tiles replaced it with the nothing they know of.
        let side = width >> 4;
        // Wider cells are never mostly sent, and counting them would open
        // thousands of region files.
        if side > MAX_KEPT_SIDE {
            self.waited |= waiting;
            return Column::default();
        }
        let (count, nearest) = self.terrain.kept(x >> 4, z >> 4, side);
        let column = (count * 2 >= (side * side) as usize)
            .then(|| nearest.and_then(|(cx, cz)| self.terrain.chunk(cx, cz)))
            .flatten()
            .map(|chunk| Column::reduced(&chunk, 0, 0, 16));
        self.waited |= waiting && column.is_none();
        column.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Columns {
        Arc::new(std::array::from_fn(|i| {
            if i % 3 == 0 {
                return Column::default();
            }
            Column {
                surface: Some(Surface {
                    height: 63 + i as i32 % 7,
                    state: BlockStateId(9 + i as u32),
                    water: (i % 2 == 0).then_some((62, BlockStateId(86))),
                    biome: (i % 5) as u16,
                }),
                spans: vec![
                    Span {
                        min: -64,
                        max: 40,
                        state: BlockStateId(1),
                        biome: 3,
                        light: 0,
                    },
                    Span {
                        min: 40,
                        max: 63 + i as i16 % 7,
                        state: BlockStateId(9),
                        biome: 7,
                        light: 0xF0,
                    },
                ]
                .into(),
            }
        }))
    }

    #[test]
    fn columns_survive_their_encoding() {
        let columns = columns();
        assert_eq!(decode(&encode(3, -4, &columns)), Some(columns));
    }

    #[test]
    fn records_of_another_version_are_ignored() {
        let mut tag = encode(0, 0, &columns());
        if let Tag::Compound(map) = &mut tag {
            map.insert("DataVersion".into(), Tag::Int(DATA_VERSION - 1));
        }
        assert_eq!(decode(&tag), None);
    }

    #[test]
    fn server_folders_are_plain_names() {
        assert_eq!(server_folder("Play.Example.net"), "play.example.net");
        assert_eq!(server_folder("play.example.net:25565"), "play.example.net");
        assert_eq!(
            server_folder("play.example.net:25566"),
            "play.example.net_25566"
        );
        assert_eq!(server_folder("[::1]:25570"), "___1__25570");
        assert_eq!(server_folder("../.."), "_");
        assert_eq!(
            level_dir(Path::new("/c"), "a", -7, "minecraft:the_nether"),
            Path::new("/c/a/-7/minecraft/the_nether")
        );
        assert_eq!(
            level_dir(Path::new("/c"), "a", 0, "mod:../../x"),
            Path::new("/c/a/0/mod/x")
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn stored_chunks_come_back_after_a_restart() {
        let dir = scratch("server-terrain");
        let columns = columns();
        {
            let terrain = ServerTerrain::new(dir.clone(), None);
            assert_eq!(terrain.chunk(-33, 5), None);
            terrain.store((-33, 5), &columns).unwrap();
        }
        let terrain = ServerTerrain::new(dir.clone(), None);
        assert_eq!(terrain.chunk(-33, 5), Some(columns.clone()));
        assert_eq!(terrain.nearest(-64, 0, 64), Some((-33, 5)));
        assert_eq!(terrain.nearest(-32, 0, 64), None);
        let mut sampler = terrain.clone().sampler();
        assert_eq!(sampler.sample(-33 * 16 + 1, 5 * 16), columns[1].surface);
        assert_eq!(sampler.column(0, 0), Column::default());
        // One chunk doesn't stand for a far cell of 64 × 64.
        assert_eq!(sampler.reduced_column(-64 * 16, 0, 1024), Column::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wide_cells_are_drawn_where_most_of_them_was_kept() {
        let dir = scratch("server-terrain-wide");
        let columns = columns();
        let terrain = ServerTerrain::new(dir.clone(), None);
        let mut sampler = terrain.clone().sampler();
        // A cell of 4 × 4 chunks, and a chunk of it kept.
        terrain.store((2, 1), &columns).unwrap();
        assert_eq!(sampler.reduced_column(0, 0, 64), Column::default());
        for (x, z) in [(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (1, 1)] {
            terrain.store((x, z), &columns).unwrap();
            assert_eq!(sampler.reduced_column(0, 0, 64), Column::default());
        }
        // Half of it: drawn from the kept chunk nearest its middle.
        terrain.store((2, 2), &columns).unwrap();
        assert_eq!(
            sampler.reduced_column(0, 0, 64),
            Column::reduced(&columns, 0, 0, 16)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Far terrain samples a cell before the server sent its chunk, then
    /// again once it is kept: nothing remembers the empty sample.
    #[test]
    fn a_chunk_kept_after_an_empty_sample_is_sampled_next_time() {
        let dir = scratch("server-terrain-late");
        let columns = columns();
        let terrain = ServerTerrain::new(dir.clone(), None);
        let mut sampler = terrain.clone().sampler();
        let mut other = sampler.fork().expect("a sampling thread's own");
        for width in [1, 4, 16] {
            assert_eq!(other.reduced_column(-16, 32, width), Column::default());
        }
        assert_eq!(other.grid(-16, 32, 1, 16), vec![Column::default(); 256]);
        // `Job::Observe` stores it on the far-terrain worker.
        sampler.observed((-1, 2), &columns);
        assert_eq!(other.grid(-16, 32, 1, 16), columns.to_vec());
        assert_eq!(
            other.reduced_column(-16, 32, 16),
            Column::reduced(&columns, 0, 0, 16)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
