//! Far terrain from a server's BlueMap: the
//! lowres tiles of the server's web map, fetched over HTTP around the player,
//! fill the far terrain where no real chunk was ever seen
//! (`server_terrain`'s store always comes first). Off unless the
//! player names the server's map (`Settings`), or lets Horizon look for it
//! (`bluemap_detect`).
//!
//! A lowres tile (`LowresTile`) is a PNG of `(size + 1)` × `2 (size + 1)`
//! pixels: the top half each column's colour, the bottom half its height
//! (green and blue, a signed 16-bit number) and block light (red). Level 1
//! has a pixel a block; each next level averages `lodFactor` × `lodFactor`
//! pixels into one (`LowresLayer.saveTile`).
//!
//! The tile layout and paths follow BlueMap's own (MIT, Copyright (c) Blue
//! (Lukas Rieger) and contributors; THIRD_PARTY_NOTICES.md): `LowresLayer`,
//! `MapSettingsSerializer` and the web app's `pathFromCoords`, at `96ae320d1d39`.
use crate::{
    biome::Biomes,
    chunk::MIN_Y,
    fast_hash::{FxHashMap, FxHashSet},
    lod::{Column, Span},
};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::{Duration, Instant, SystemTime},
};

/// Downloaded tiles kept on disk per server, at most.
const DISK_CAP: u64 = 256 << 20;
/// A cached tile older than this is fetched again (and drawn meanwhile).
const MAX_AGE: Duration = Duration::from_secs(3 * 24 * 3600);
/// One request at a time, this far apart at least.
const REQUEST_GAP: Duration = Duration::from_millis(100);
/// After a failed request the server is left alone this long, doubling.
const BACKOFF: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(600);
/// Decoded tiles kept in memory, at most this many bytes of pixels (a
/// 500-block tile is about 2 MiB) and this many tiles.
const MAX_TILE_BYTES: usize = 192 << 20;
const MAX_TILES: usize = 2048;
const MAX_CHUNKS: usize = 4096;
const MAX_QUEUE: usize = 4096;
/// Arrived data is handed to far terrain at most this often (`take_changed`),
/// so a burst of tiles is sampled again once.
const CHANGE_GAP: Duration = Duration::from_millis(250);
/// How far a column reaches under its lowest neighbour's top: its sides meet
/// theirs, and the edge of the map's data shows no wall down to the void.
const SKIRT: i32 = 8;

/// Where the player names the servers' maps (`Settings::servers`), beside
/// `horizon-graphics.txt`: kept out of the graphics settings, which shared
/// instances and modpacks carry, as a map the client asks for tiles around
/// the player (where they stand, which server) is the player's to name, not
/// a pack's.
pub const SERVERS_FILE: &str = "horizon-bluemap.txt";
/// The player's BlueMap settings: `blueMapRadius:<chunks>` (or `<blocks>b`)
/// and `blueMapAutoDetect:true` in `horizon-graphics.txt`, and
/// `blueMap:<server address> <map URL> [<level>=<map id> ...]` per server in
/// `SERVERS_FILE`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// How far around the player tiles are downloaded, in chunks; 0 uses
    /// `DEFAULT_RADIUS`.
    pub radius: u32,
    /// Look for the map of a server no `blueMap:` line names.
    pub auto_detect: bool,
    pub servers: Vec<Server>,
}
pub const DEFAULT_RADIUS: u32 = 64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub address: String,
    pub url: String,
    /// `(level, map id)`: which of the server's maps draws which level;
    /// unnamed levels are guessed from the map ids (`pick_map`).
    pub maps: Vec<(String, String)>,
}
impl Settings {
    /// `blueMapRadius:`'s value: chunks, or blocks with a `b` (`2048b`).
    pub fn parse_radius(value: &str) -> Option<u32> {
        let value = value.trim().to_ascii_lowercase();
        let blocks = value
            .strip_suffix("blocks")
            .or_else(|| value.strip_suffix('b'));
        let chunks = match blocks {
            Some(blocks) => blocks.trim().parse::<u32>().ok()?.div_ceil(16),
            None => value
                .strip_suffix("chunks")
                .unwrap_or(&value)
                .trim()
                .parse()
                .ok()?,
        };
        Some(chunks.min(4096))
    }
    /// A `blueMap:` line's value.
    pub fn parse_server(value: &str) -> Option<Server> {
        let mut words = value.split_whitespace();
        let address = words.next()?.to_owned();
        let url = words.next()?.to_owned();
        let maps = words
            .filter_map(|w| w.split_once('='))
            .map(|(level, map)| (level_key(level), map.to_owned()))
            .collect();
        Some(Server { address, url, maps })
    }
    /// The `blueMap:` lines of `SERVERS_FILE`.
    pub fn parse_servers(text: &str) -> Vec<Server> {
        // Notepad's "UTF-8 with BOM" would hide the first line.
        text.trim_start_matches('\u{feff}')
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(key, _)| key.trim() == "blueMap")
            .filter_map(|(_, value)| Self::parse_server(value))
            .collect()
    }
    /// What goes in `horizon-graphics.txt` (the servers don't).
    pub fn serialize(&self) -> String {
        format!(
            "blueMapRadius:{}\nblueMapAutoDetect:{}\n",
            self.radius, self.auto_detect
        )
    }
    /// The settings of the server at `address`, as typed to join it.
    pub fn server(&self, address: &str) -> Option<&Server> {
        let folder = crate::server_terrain::server_folder(address);
        self.servers
            .iter()
            .find(|s| crate::server_terrain::server_folder(&s.address) == folder)
    }
    pub fn radius_blocks(&self) -> i32 {
        self.radius_chunks() as i32 * 16
    }
    pub fn radius_chunks(&self) -> u32 {
        if self.radius == 0 {
            DEFAULT_RADIUS
        } else {
            self.radius
        }
    }
}

/// `nether` → `minecraft:the_nether`, `overworld` → `minecraft:overworld`.
fn level_key(level: &str) -> String {
    match level {
        "nether" => "minecraft:the_nether".into(),
        "end" => "minecraft:the_end".into(),
        _ if level.contains(':') => level.into(),
        _ => format!("minecraft:{level}"),
    }
}

/// The map drawing `level` among the server's map ids: the player's choice,
/// else the first id naming the level's dimension (`world_nether`,
/// `end`...), the overworld's being the first that names neither.
pub fn pick_map(maps: &[String], level: &str, chosen: &[(String, String)]) -> Option<String> {
    if let Some((_, map)) = chosen.iter().find(|(l, _)| *l == level) {
        return Some(map.clone());
    }
    let named = |m: &&String, word: &str| m.to_ascii_lowercase().contains(word);
    match level {
        "minecraft:the_nether" => maps.iter().find(|m| named(m, "nether")),
        "minecraft:the_end" => maps.iter().find(|m| named(m, "end")),
        "minecraft:overworld" => maps
            .iter()
            .find(|m| !named(m, "nether") && !named(m, "end")),
        _ => None,
    }
    .cloned()
}

/// The web app's `settings.json`: the map ids and where their data lies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub maps: Vec<String>,
    pub data_root: String,
}
impl Root {
    pub fn parse(json: &[u8]) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_slice(json).ok()?;
        let maps = v
            .get("maps")?
            .as_array()?
            .iter()
            .filter_map(|m| m.as_str().map(str::to_owned))
            .collect();
        let data_root = v
            .get("mapDataRoot")
            .and_then(|r| r.as_str())
            .unwrap_or("maps")
            .trim_matches('/')
            .to_owned();
        Some(Self { maps, data_root })
    }
}

/// A map's `settings.json`, its lowres part (`MapSettingsSerializer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lowres {
    pub tile: [i32; 2],
    pub factor: i32,
    pub levels: u8,
}
impl Lowres {
    pub fn parse(json: &[u8]) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_slice(json).ok()?;
        let lowres = v.get("lowres")?;
        let size = lowres.get("tileSize").and_then(|s| s.as_array());
        let axis = |i: usize| {
            size.and_then(|s| s.get(i)?.as_i64()).map_or(Some(32), |n| {
                i32::try_from(n).ok().filter(|n| (1..=1024).contains(n))
            })
        };
        let factor = lowres
            .get("lodFactor")
            .and_then(|f| f.as_i64())
            .unwrap_or(5);
        let levels = lowres.get("lodCount").and_then(|c| c.as_i64()).unwrap_or(3);
        let lowres = Self {
            tile: [axis(0)?, axis(1)?],
            factor: i32::try_from(factor)
                .ok()
                .filter(|f| (2..=64).contains(f))?,
            levels: u8::try_from(levels).ok().filter(|c| (1..=8).contains(c))?,
        };
        // A tile packs `factor` of the level below on each axis: smaller ones
        // have no pixel for each, and the coarsest must span the world, no
        // more (block coordinates stay far from overflowing).
        let coarsest = lowres.factor.checked_pow(u32::from(lowres.levels) - 1)?;
        lowres
            .tile
            .iter()
            .all(|&size| {
                size >= lowres.factor && size.checked_mul(coarsest).is_some_and(|b| b <= 1 << 26)
            })
            .then_some(lowres)
    }
    /// How many decoded tiles fit `MAX_TILE_BYTES`.
    fn tiles_in_memory(&self) -> usize {
        let pixels = (self.tile[0] as usize + 1) * (self.tile[1] as usize + 1);
        (MAX_TILE_BYTES / (pixels * std::mem::size_of::<Pixel>())).max(16)
    }
    /// Blocks a pixel of `level` (1 = finest) spans, about.
    pub fn scale(&self, level: u8) -> i32 {
        self.factor.saturating_pow(u32::from(level) - 1)
    }
    /// The coarsest level whose pixels are no wider than `width` blocks.
    pub fn level_for(&self, width: i32) -> u8 {
        (1..=self.levels)
            .rev()
            .find(|&l| self.scale(l) <= width)
            .unwrap_or(1)
    }
    /// The far-terrain cell widths drawn from `level`'s pixels, `(min, max)`:
    /// up to a chunk from level 1 (`BlueMap::chunk`), wider from `level_for`.
    pub fn widths(&self, level: u8) -> (i32, i32) {
        let next = if level >= self.levels {
            i32::MAX
        } else {
            self.scale(level + 1) - 1
        };
        if level <= 1 {
            (1, next.max(16))
        } else {
            (self.scale(level).max(17), next)
        }
    }
    /// The tile of `level` holding block coordinate `block` on one axis, and
    /// its pixel. Each level packs `factor` tiles of the one below into one,
    /// `size / factor` pixels a tile (`LowresLayer.saveTile`): a pixel past
    /// those (`size % factor` of them) is drawn with the last.
    pub fn locate(&self, axis: usize, level: u8, block: i32) -> (i32, usize) {
        let size = self.tile[axis];
        let groups = (size / self.factor).max(1);
        let (mut tile, mut pixel) = (block.div_euclid(size), block.rem_euclid(size));
        for _ in 1..level {
            let group = (pixel / self.factor).min(groups - 1);
            pixel = tile.rem_euclid(self.factor) * groups + group;
            tile = tile.div_euclid(self.factor);
        }
        (tile, pixel as usize)
    }
    /// The blocks of one axis a tile of `level` covers, `[min, max)`.
    pub fn span(&self, axis: usize, level: u8, tile: i32) -> (i32, i32) {
        let blocks = self.tile[axis].saturating_mul(self.scale(level));
        (
            tile.saturating_mul(blocks),
            (tile + 1).saturating_mul(blocks),
        )
    }
}

/// `pathFromCoords`: `(12, -3)` → `x1/2/z-3`.
pub fn tile_path(x: i32, z: i32) -> String {
    let digits = |n: i32| {
        let mut out = String::new();
        if n < 0 {
            out.push('-');
        }
        for c in n.unsigned_abs().to_string().chars() {
            out.push(c);
            out.push('/');
        }
        out
    };
    let mut path = format!("x{}z{}", digits(x), digits(z));
    path.pop();
    path
}

/// One column of a lowres tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel {
    /// ARGB, straight alpha; 0 alpha: nothing was rendered there.
    pub color: u32,
    /// The highest block BlueMap drew.
    pub height: i16,
    pub block_light: u8,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Tile {
    width: usize,
    pixels: Vec<Pixel>,
}
impl Tile {
    pub fn pixel(&self, x: usize, z: usize) -> Pixel {
        self.pixels[z * self.width + x]
    }
}

/// A lowres tile's PNG (gzipped too, as a misconfigured web server may send
/// it), `None` if it isn't one of `lowres`'s size.
pub fn decode_tile(bytes: &[u8], lowres: &Lowres) -> Option<Tile> {
    let unzipped;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        unzipped = horizon_world::compression::Compression::Gzip
            .decode(bytes, 8 << 20)
            .ok()?;
        &unzipped[..]
    } else {
        bytes
    };
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = (lowres.tile[0] as usize + 1, lowres.tile[1] as usize + 1);
    let info = reader.info();
    if info.width as usize != width || info.height as usize != height * 2 {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let channels = frame.color_type.samples();
    let rgba = |i: usize| -> [u8; 4] {
        let p = &buf[i * channels..(i + 1) * channels];
        match channels {
            4 => [p[0], p[1], p[2], p[3]],
            3 => [p[0], p[1], p[2], 255],
            2 => [p[0], p[0], p[0], p[1]],
            _ => [p[0], p[0], p[0], 255],
        }
    };
    let pixels = (0..width * height)
        .map(|i| {
            let [r, g, b, a] = rgba(i);
            let [light, hi, lo, _] = rgba(i + width * height);
            Pixel {
                color: u32::from_be_bytes([a, r, g, b]),
                // No world reaches past ±2048 blocks (`DimensionType`).
                height: i16::from_be_bytes([hi, lo]).clamp(-2048, 2047),
                block_light: light,
            }
        })
        .collect();
    Some(Tile { width, pixels })
}

/// What a lookup found: data, data on its way, or nothing to wait for.
#[derive(Clone, Debug, PartialEq)]
pub enum Found<T> {
    Ready(T),
    Waiting,
    Absent,
}

/// Block states a map colour is drawn with: the nearest of a few natural
/// blocks' colours as BlueMap renders them (their texture's mean, tinted with
/// each biome's grass, foliage and water colours).
pub struct Palette {
    candidates: Vec<Candidate>,
    under: BlockStateId,
    deep: BlockStateId,
    matches: Mutex<FxHashMap<u32, usize>>,
}
#[derive(Clone, Copy)]
struct Candidate {
    rgb: [f32; 3],
    top: BlockStateId,
    /// The few blocks under the top one (dirt under grass).
    under: Option<BlockStateId>,
    biome: u16,
    water: bool,
}
/// A biome's tints, for `Palette::new`.
pub struct Tints {
    pub id: u16,
    pub grass: u32,
    pub foliage: u32,
    pub water: u32,
}
/// Natural and common building blocks, with BlueMap's colour of each, about.
const BLOCKS: &[(&str, u32)] = &[
    ("stone", 0x7E7E7E),
    ("deepslate", 0x505053),
    ("dirt", 0x866043),
    ("podzol", 0x5C3F18),
    ("mycelium", 0x6F6369),
    ("sand", 0xDBCFA3),
    ("red_sand", 0xBE6621),
    ("gravel", 0x847F7F),
    ("clay", 0xA0A6B3),
    ("snow_block", 0xF5FAFA),
    ("ice", 0x91B7FD),
    ("terracotta", 0x985E44),
    ("orange_terracotta", 0xA25426),
    ("yellow_terracotta", 0xBA8523),
    ("white_terracotta", 0xD2B2A1),
    ("brown_terracotta", 0x4D3324),
    ("red_terracotta", 0x8F3D2F),
    ("light_gray_terracotta", 0x876B62),
    ("moss_block", 0x596E2D),
    ("mud", 0x3C3A3D),
    ("netherrack", 0x622626),
    ("crimson_nylium", 0x831F1F),
    ("warped_nylium", 0x2B7265),
    ("soul_sand", 0x513E33),
    ("basalt", 0x505155),
    ("blackstone", 0x2A2328),
    ("magma_block", 0x8E3F1F),
    ("lava", 0xCF5B13),
    ("end_stone", 0xDBDEA1),
    ("obsidian", 0x0F0A18),
    ("oak_planks", 0xA2834F),
    ("spruce_planks", 0x735531),
    ("dark_oak_planks", 0x432B14),
    ("bricks", 0x976253),
    ("smooth_stone", 0x9F9F9F),
    ("quartz_block", 0xEBE5DE),
    ("white_concrete", 0xCFD5D6),
    ("light_gray_concrete", 0x7D7D73),
    ("gray_concrete", 0x373A3E),
    ("black_concrete", 0x080A0F),
    ("red_concrete", 0x8E2121),
    ("blue_concrete", 0x2D2F8F),
    ("spruce_leaves", 0x304C30),
    ("birch_leaves", 0x40532A),
];
/// The mean grey of grass_block_top, the leaves and water_still, which the
/// biome colours tint.
const GRASS_GREY: f32 = 0.6;
const LEAVES_GREY: f32 = 0.5;
const WATER_GREY: f32 = 0.7;

fn rgb(color: u32) -> [f32; 3] {
    [16, 8, 0].map(|s| ((color >> s) & 255) as f32)
}
impl Palette {
    pub fn new(
        state: impl Fn(&str) -> Option<BlockStateId>,
        biomes: &[Tints],
        plains: u16,
    ) -> Self {
        let block = |name: &str| state(&format!("minecraft:{name}"));
        let mut candidates = Vec::new();
        let dirt = block("dirt");
        for &(name, color) in BLOCKS {
            if let Some(top) = block(name) {
                let under = matches!(name, "podzol" | "mycelium" | "snow_block").then_some(dirt);
                candidates.push(Candidate {
                    rgb: rgb(color),
                    top,
                    under: under.flatten(),
                    biome: plains,
                    water: false,
                });
            }
        }
        let tinted = |tint: u32, grey: f32| rgb(tint).map(|c| c * grey);
        let mut seen = FxHashSet::default();
        for b in biomes {
            for (state, color, under, water) in [
                (
                    block("grass_block"),
                    tinted(b.grass, GRASS_GREY),
                    dirt,
                    false,
                ),
                (
                    block("oak_leaves"),
                    tinted(b.foliage, LEAVES_GREY),
                    None,
                    false,
                ),
                (block("water"), tinted(b.water, WATER_GREY), None, true),
            ] {
                let key = (color.map(|c| c as u32), water, under.is_some());
                if let Some(top) = state
                    && seen.insert(key)
                {
                    candidates.push(Candidate {
                        rgb: color,
                        top,
                        under,
                        biome: b.id,
                        water,
                    });
                }
            }
        }
        Self {
            candidates,
            under: block("sand").unwrap_or(BlockStateId(0)),
            deep: block("stone").unwrap_or(BlockStateId(0)),
            matches: Mutex::default(),
        }
    }
    pub fn from_game(registry: &BlockRegistry, biomes: &Biomes) -> Self {
        let tints: Vec<_> = biomes
            .list
            .iter()
            .enumerate()
            .map(|(id, b)| Tints {
                id: id as u16,
                grass: b.grass,
                foliage: b.foliage,
                water: b.water,
            })
            .collect();
        Self::new(
            |name| registry.default_state(name),
            &tints,
            biomes.id("minecraft:plains"),
        )
    }
    /// `matches`: `self.matches`, locked once by the caller for many pixels.
    fn nearest(&self, matches: &mut FxHashMap<u32, usize>, color: u32) -> Option<Candidate> {
        let key = color & 0xFF_FFFF;
        if matches.len() > 1 << 16 {
            matches.clear();
        }
        let index = *matches.entry(key).or_insert_with(|| {
            let want = rgb(key);
            let distance = |c: &Candidate| {
                // Weighted towards green, as the eye is (a "redmean"-ish norm).
                let d = [0, 1, 2].map(|i| c.rgb[i] - want[i]);
                2.0 * d[0] * d[0] + 4.0 * d[1] * d[1] + 3.0 * d[2] * d[2]
            };
            (0..self.candidates.len())
                .min_by(|&a, &b| {
                    distance(&self.candidates[a]).total_cmp(&distance(&self.candidates[b]))
                })
                .unwrap_or(usize::MAX)
        });
        self.candidates.get(index).copied()
    }
    /// A pixel as a far-terrain column: its top block at its height, a few
    /// blocks under it, then stone; water over sand. `low`: the lowest top
    /// of it and its neighbours; the stone ends `SKIRT` under it, not at the
    /// bottom of the world (a wall where the map's data ends).
    pub fn column(&self, pixel: Pixel, low: i32) -> Column {
        self.column_in(&mut self.matches.lock().unwrap(), pixel, low)
    }
    /// Columns of many pixels, `(pixel, low)` each, matched under one lock.
    fn columns<const N: usize>(&self, pixels: [Option<(Pixel, i32)>; N]) -> [Column; N] {
        let mut matches = self.matches.lock().unwrap();
        pixels.map(|p| {
            p.map_or_else(Column::default, |(p, low)| {
                self.column_in(&mut matches, p, low)
            })
        })
    }
    fn column_in(&self, matches: &mut FxHashMap<u32, usize>, pixel: Pixel, low: i32) -> Column {
        if pixel.color >> 24 == 0 {
            return Column::default();
        }
        let Some(c) = self.nearest(matches, pixel.color) else {
            return Column::default();
        };
        let top = i32::from(pixel.height) + 1;
        let bottom = low.min(top) - SKIRT;
        let light = 240 | u16::from(pixel.block_light.min(15)) << 12;
        let mut column = Column::default();
        let mut push = |min: i32, max: i32, state: BlockStateId| {
            let (min, max) = (min.max(MIN_Y), max.max(MIN_Y));
            if min < max {
                column.push(Span {
                    min: min as i16,
                    max: max as i16,
                    state,
                    biome: c.biome,
                    light,
                });
            }
        };
        if c.water {
            let floor = top - 3;
            push(bottom.min(floor - 4), floor - 3, self.deep);
            push(floor - 3, floor, self.under);
            push(floor, top, c.top);
            column.surface = Some(crate::lod::Surface {
                height: floor,
                state: self.under,
                water: Some((top, c.top)),
                biome: c.biome,
            });
        } else {
            let under = c.under.unwrap_or(c.top);
            push(bottom.min(top - 5), top - 4, self.deep);
            push(top - 4, top - 1, under);
            push(top - 1, top, c.top);
            column.surface = Some(crate::lod::Surface {
                height: top,
                state: c.top,
                water: None,
                biome: c.biome,
            });
        }
        column
    }
}

type Key = (u8, i32, i32);
type Columns = Arc<[Column; 256]>;
/// The level's map: its id, its folder on the server and its lowres layout.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MapInfo {
    id: String,
    root: String,
    lowres: Lowres,
}
impl Slot {
    fn found(&self) -> Found<Arc<Tile>> {
        match self {
            Slot::Ready(tile) => Found::Ready(tile.clone()),
            Slot::Missing | Slot::Unreached => Found::Absent,
            Slot::Waiting => Found::Waiting,
        }
    }
}
enum Slot {
    Ready(Arc<Tile>),
    Missing,
    Waiting,
    /// Not cached, and out of reach or the server down when looked up:
    /// wanted when that changes (`State::reach`), drawn without meanwhile.
    Unreached,
}
/// What the map knows of the server, and what it wants from it.
#[derive(Default)]
struct State {
    map: Option<Arc<MapInfo>>,
    tiles: FxHashMap<Key, Slot>,
    chunks: FxHashMap<(i32, i32), Columns>,
    queue: FxHashSet<Key>,
    center: Option<[i32; 2]>,
    /// Where `center` was when the tiles come into reach were last wanted.
    anchor: Option<[i32; 2]>,
    /// The server failed the last request: nothing waits for it until then.
    offline_until: Option<Instant>,
    /// What arrived (`take_changed`).
    changed: Vec<Change>,
    /// When `take_changed` last handed something over.
    taken_at: Option<Instant>,
    /// Bumped when a tile arrives or the map changes: a chunk built from
    /// what was there before isn't kept (`BlueMap::chunk`).
    generation: u64,
}
/// Data arrived for the blocks `area` `[x0, z0, x1, z1)`, drawn in far-terrain
/// cells `widths` `(min, max)` blocks wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub area: [i32; 4],
    pub widths: (i32, i32),
}
struct Inner {
    /// The map's web address; unset while it is looked for at `probes`.
    base: OnceLock<String>,
    probes: Vec<String>,
    level: String,
    chosen: Vec<(String, String)>,
    dir: PathBuf,
    radius: i32,
    palette: Palette,
    state: Mutex<State>,
    wake: Condvar,
}
/// One level's BlueMap: tiles are read from the disk cache on the far-terrain
/// threads and downloaded on a worker of its own.
pub struct BlueMap {
    inner: Arc<Inner>,
}

/// `https://map.example.net/#world:0:64:0...` → `https://map.example.net`.
pub fn base_url(url: &str) -> String {
    let url = url.trim();
    let url = url.split(['#', '?']).next().unwrap_or(url);
    let url = url.strip_suffix("index.html").unwrap_or(url);
    url.trim_end_matches('/').to_owned()
}

impl BlueMap {
    /// The map of `level` at `url`, its tiles cached in `dir`; nothing is
    /// downloaded until `start`.
    pub fn new(
        url: &str,
        level: &str,
        chosen: Vec<(String, String)>,
        dir: PathBuf,
        radius_blocks: i32,
        palette: Palette,
    ) -> Self {
        let base = OnceLock::from(base_url(url));
        Self::build(base, Vec::new(), level, chosen, dir, radius_blocks, palette)
    }
    /// The map of `level`, looked for at `probes` (`bluemap_detect`) first:
    /// lookups find nothing new until it is found.
    pub fn detect(
        probes: Vec<String>,
        level: &str,
        dir: PathBuf,
        radius_blocks: i32,
        palette: Palette,
    ) -> Self {
        let probes = probes.iter().map(|p| base_url(p)).collect();
        let base = OnceLock::new();
        Self::build(base, probes, level, Vec::new(), dir, radius_blocks, palette)
    }
    /// The map from where `source` says.
    pub fn from_source(
        source: crate::bluemap_detect::Source,
        level: &str,
        dir: PathBuf,
        radius_blocks: i32,
        palette: Palette,
    ) -> Self {
        use crate::bluemap_detect::Source;
        match source {
            Source::Url { url, maps } => Self::new(&url, level, maps, dir, radius_blocks, palette),
            Source::Detect(probes) => Self::detect(probes, level, dir, radius_blocks, palette),
        }
    }
    fn build(
        base: OnceLock<String>,
        probes: Vec<String>,
        level: &str,
        chosen: Vec<(String, String)>,
        dir: PathBuf,
        radius_blocks: i32,
        palette: Palette,
    ) -> Self {
        let inner = Arc::new(Inner {
            base,
            probes,
            level: level.into(),
            chosen,
            dir,
            radius: radius_blocks,
            palette,
            state: Mutex::default(),
            wake: Condvar::new(),
        });
        let map = inner.map_info(|_, name| std::fs::read(inner.dir.join(name)).ok());
        inner.state.lock().unwrap().map = map.map(Arc::new);
        Self { inner }
    }
    /// Downloads what lookups want, on a thread that ends with the map.
    pub fn start(&self) {
        let inner = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("BlueMap".into())
            .spawn(move || fetch(inner));
        if let Err(e) = spawned {
            log::warn!("Horizon: BlueMap worker: {e}");
        }
    }
    /// The player's block position: tiles are downloaded around it. Called
    /// every frame: it only wants the tiles come into reach, once the player
    /// has moved a step, and changes no far terrain (their data will).
    pub fn set_center(&self, x: i32, z: i32) {
        let mut state = self.inner.state.lock().unwrap();
        state.center = Some([x, z]);
        let step = (self.inner.radius / 4).max(64);
        let moved = state
            .anchor
            .is_none_or(|[ax, az]| (x - ax).abs() >= step || (z - az).abs() >= step);
        if moved {
            state.anchor = Some([x, z]);
            state.reach(self.inner.radius, &self.inner.wake);
        }
    }
    /// What arrived since the last call, at most every `CHANGE_GAP`: far
    /// terrain samples it again.
    pub fn take_changed(&self) -> Vec<Change> {
        let mut state = self.inner.state.lock().unwrap();
        if state.changed.is_empty() || state.taken_at.is_some_and(|t| t.elapsed() < CHANGE_GAP) {
            return Vec::new();
        }
        state.taken_at = Some(Instant::now());
        std::mem::take(&mut state.changed)
    }
    /// A chunk's columns from the finest level.
    pub fn chunk(&self, cx: i32, cz: i32) -> Found<Columns> {
        let (map, generation) = {
            let state = self.inner.state.lock().unwrap();
            if let Some(columns) = state.chunks.get(&(cx, cz)) {
                return Found::Ready(columns.clone());
            }
            let Some(map) = state.map.clone() else {
                return Found::Absent;
            };
            (map, state.generation)
        };
        // The chunk's pixels and a ring around (their tops bound its depth).
        let mut last = None;
        let mut ring = [None; 18 * 18];
        for (i, slot) in ring.iter_mut().enumerate() {
            let (dx, dz) = ((i % 18) as i32 - 1, (i / 18) as i32 - 1);
            let inside = (0..16).contains(&dx) && (0..16).contains(&dz);
            match self
                .inner
                .pixel(&map, &mut last, 1, cx * 16 + dx, cz * 16 + dz)
            {
                Found::Ready(pixel) => *slot = Some(pixel).filter(|p| p.color >> 24 != 0),
                // A neighbour on its way only bounds the depth when here.
                Found::Waiting if inside => return Found::Waiting,
                _ => {}
            }
        }
        if !ring
            .iter()
            .enumerate()
            .any(|(i, p)| p.is_some() && (1..17).contains(&(i % 18)) && (1..17).contains(&(i / 18)))
        {
            return Found::Absent;
        }
        let top = |i: usize| ring[i].map(|p| i32::from(p.height) + 1);
        let pixels: [_; 256] = std::array::from_fn(|i| {
            let at = (i / 16 + 1) * 18 + i % 16 + 1;
            let pixel = ring[at]?;
            let low = [at - 1, at + 1, at - 18, at + 18]
                .into_iter()
                .filter_map(top)
                .fold(i32::from(pixel.height) + 1, i32::min);
            Some((pixel, low))
        });
        let columns = Arc::new(self.inner.palette.columns(pixels));
        let mut state = self.inner.state.lock().unwrap();
        // Built from tiles since replaced: drawn this once, not kept.
        if state.generation != generation {
            return Found::Ready(columns);
        }
        if state.chunks.len() >= MAX_CHUNKS {
            state.chunks.clear();
        }
        state.chunks.insert((cx, cz), columns.clone());
        Found::Ready(columns)
    }
    /// The column of a far-terrain cell `width` blocks wide, from the
    /// coarsest level fine enough for it.
    pub fn reduced_column(&self, x: i32, z: i32, width: i32) -> Found<Column> {
        let Some(map) = self.inner.state.lock().unwrap().map.clone() else {
            return Found::Absent;
        };
        let level = map.lowres.level_for(width);
        let (x, z) = (x + width / 2, z + width / 2);
        let mut last = None;
        let pixel = match self.inner.pixel(&map, &mut last, level, x, z) {
            Found::Ready(pixel) => pixel,
            Found::Waiting => return Found::Waiting,
            Found::Absent => return Found::Absent,
        };
        // The neighbouring cells' middles bound its depth.
        let low = [(-width, 0), (width, 0), (0, -width), (0, width)]
            .into_iter()
            .filter_map(
                |(dx, dz)| match self.inner.pixel(&map, &mut last, level, x + dx, z + dz) {
                    Found::Ready(p) if p.color >> 24 != 0 => Some(i32::from(p.height) + 1),
                    _ => None,
                },
            )
            .fold(i32::from(pixel.height) + 1, i32::min);
        Found::Ready(self.inner.palette.column(pixel, low))
    }
}

/// The tile a lookup used last (a chunk's 256 columns mostly share one).
type Last = Option<(Key, Option<Arc<Tile>>)>;

impl State {
    fn announce(&mut self, change: Change) {
        if self.changed.len() >= 256 {
            let all = self.changed.drain(..).fold(change, |a, b| Change {
                area: [
                    a.area[0].min(b.area[0]),
                    a.area[1].min(b.area[1]),
                    a.area[2].max(b.area[2]),
                    a.area[3].max(b.area[3]),
                ],
                widths: (a.widths.0.min(b.widths.0), a.widths.1.max(b.widths.1)),
            });
            self.changed.push(all);
        } else {
            self.changed.push(change);
        }
    }
    /// Past the world border (30 million blocks), with room to spare.
    const WORLD: i32 = 1 << 26;
    /// Everything may differ (a new map, none): far terrain samples again
    /// all it drew from the map, cached tiles reach past the download radius.
    fn announce_all(&mut self) {
        self.announce(Change {
            // The world's width, with room for `Lod::refresh` to widen it.
            area: [-Self::WORLD, -Self::WORLD, Self::WORLD, Self::WORLD],
            widths: (1, i32::MAX),
        });
    }
    /// How far a tile is from the player, squared, if in reach.
    fn near(&self, lowres: &Lowres, key: Key, radius: i32) -> Option<i64> {
        let [x, z] = self.center?;
        let (x0, x1) = lowres.span(0, key.0, key.1);
        let (z0, z1) = lowres.span(1, key.0, key.2);
        let dx = (x0 - x).max(x - x1).max(0);
        let dz = (z0 - z).max(z - z1).max(0);
        (dx.max(dz) <= radius).then(|| i64::from(dx).pow(2) + i64::from(dz).pow(2))
    }
    fn online(&self) -> bool {
        self.offline_until.is_none_or(|t| t <= Instant::now())
    }
    /// Wants `key` from the server, if it can: else it is `Unreached`.
    fn want(&mut self, key: Key, wake: &Condvar) -> bool {
        let queued = self.queue.len() < MAX_QUEUE || self.queue.contains(&key);
        if queued {
            self.queue.insert(key);
            wake.notify_one();
        }
        self.tiles.insert(
            key,
            if queued {
                Slot::Waiting
            } else {
                Slot::Unreached
            },
        );
        queued
    }
    /// Wants the `Unreached` tiles now in reach, the server answering.
    fn reach(&mut self, radius: i32, wake: &Condvar) {
        let Some(map) = self.map.clone().filter(|_| self.online()) else {
            return;
        };
        let reached: Vec<Key> = self
            .tiles
            .iter()
            .filter(|(key, slot)| {
                matches!(slot, Slot::Unreached) && self.near(&map.lowres, **key, radius).is_some()
            })
            .map(|(key, _)| *key)
            .collect();
        for key in reached {
            if !self.want(key, wake) {
                break;
            }
        }
    }
}

impl Inner {
    /// The level's map from settings files, cached or downloaded: `get`
    /// takes a file's path on the server and its name in the cache (the
    /// server's names never pick a path here).
    fn map_info(&self, mut get: impl FnMut(&str, &str) -> Option<Vec<u8>>) -> Option<MapInfo> {
        let root = Root::parse(&get("settings.json", "settings.json")?)?;
        let id = pick_map(&root.maps, &self.level, &self.chosen)?;
        let lowres = Lowres::parse(&get(
            &format!("{}/{id}/settings.json", root.data_root),
            &format!("{}.json", crate::server_terrain::server_folder(&id)),
        )?)?;
        Some(MapInfo {
            id,
            root: root.data_root,
            lowres,
        })
    }
    fn tile_file(&self, map: &MapInfo, (level, x, z): Key) -> PathBuf {
        self.dir
            .join(crate::server_terrain::server_folder(&map.id))
            .join(level.to_string())
            .join(format!("{x}_{z}"))
    }
    fn pixel(&self, map: &MapInfo, last: &mut Last, level: u8, x: i32, z: i32) -> Found<Pixel> {
        let level = level.min(map.lowres.levels);
        let (tx, px) = map.lowres.locate(0, level, x);
        let (tz, pz) = map.lowres.locate(1, level, z);
        let key = (level, tx, tz);
        let tile = match last {
            Some((k, tile)) if *k == key => tile.clone(),
            _ => {
                let tile = match self.tile(map, key) {
                    Found::Ready(tile) => Some(tile),
                    Found::Waiting => return Found::Waiting,
                    Found::Absent => None,
                };
                *last = Some((key, tile.clone()));
                tile
            }
        };
        tile.map_or(Found::Absent, |t| Found::Ready(t.pixel(px, pz)))
    }
    /// A tile from memory, else the disk cache, else wanted from the server
    /// when it is in reach.
    fn tile(&self, map: &MapInfo, key: Key) -> Found<Arc<Tile>> {
        // Tiles are of the map in `state`: a lookup of an older one (new
        // settings came meanwhile) would read them with the wrong layout.
        let current = |state: &State| {
            state
                .map
                .as_deref()
                .is_some_and(|now| std::ptr::eq(now, map))
        };
        {
            let state = self.state.lock().unwrap();
            if !current(&state) {
                return Found::Absent;
            }
            if let Some(slot) = state.tiles.get(&key) {
                return slot.found();
            }
        }
        let cached = read_cached(&self.tile_file(map, key), &map.lowres);
        let mut state = self.state.lock().unwrap();
        if !current(&state) {
            return Found::Absent;
        }
        // Another thread, or the download, put it in meanwhile: theirs.
        if let Some(slot) = state.tiles.get(&key) {
            return slot.found();
        }
        let ready = state
            .tiles
            .values()
            .filter(|slot| matches!(slot, Slot::Ready(_)))
            .count();
        if state.tiles.len() >= MAX_TILES || ready >= map.lowres.tiles_in_memory() {
            state
                .tiles
                .retain(|_, slot| matches!(slot, Slot::Waiting | Slot::Unreached));
            if state.tiles.len() >= MAX_TILES {
                state.tiles.retain(|_, slot| matches!(slot, Slot::Waiting));
            }
        }
        let reachable = state.near(&map.lowres, key, self.radius).is_some() && state.online();
        match cached {
            Some((tile, fresh)) => {
                if !fresh && reachable && state.queue.len() < MAX_QUEUE && state.queue.insert(key) {
                    self.wake.notify_one();
                }
                let tile = tile.map(Arc::new);
                let slot = tile.clone().map_or(Slot::Missing, Slot::Ready);
                state.tiles.insert(key, slot);
                tile.map_or(Found::Absent, Found::Ready)
            }
            None if reachable && state.want(key, &self.wake) => Found::Waiting,
            None => {
                state.tiles.insert(key, Slot::Unreached);
                Found::Absent
            }
        }
    }
}
/// Writes a file whole or not at all: a crash or a full disk mid-write
/// leaves the old one (or none), never half a tile.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}
/// A cached tile (`None` inside: the server has none there) and whether it
/// is fresh; `None`: not cached.
fn read_cached(file: &Path, lowres: &Lowres) -> Option<(Option<Tile>, bool)> {
    let fresh = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < MAX_AGE)
    };
    // One that doesn't decode (cut short, or of an older `tileSize`) is
    // fetched again: the worker keeps only those that do.
    let png = file.with_extension("png");
    if let Ok(bytes) = std::fs::read(&png) {
        return decode_tile(&bytes, lowres).map(|tile| (Some(tile), fresh(&png)));
    }
    let none = file.with_extension("none");
    none.exists().then(|| (None, fresh(&none)))
}

/// The download worker's own state.
struct Worker {
    agent: ureq::Agent,
    backoff: Duration,
    /// Bytes in the disk cache, counted on the first download.
    disk: Option<u64>,
    /// The settings were fetched on this visit.
    settings: bool,
    /// Candidates tried while looking for the map, and the right to look.
    probed: usize,
    claim: Option<crate::bluemap_detect::Claim>,
    /// Nothing more to do: no map was found.
    done: bool,
}

/// The download worker: settings first, then the wanted tile nearest the
/// player, one request at a time. A failure leaves the server alone for a
/// while, and lookups stop waiting for it meanwhile.
fn fetch(inner: Weak<Inner>) {
    let mut worker = Worker {
        agent: ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
            .build(),
        backoff: BACKOFF,
        disk: None,
        settings: false,
        probed: 0,
        claim: None,
        done: false,
    };
    while !worker.done {
        let Some(inner) = inner.upgrade() else { return };
        let pause = worker.step(&inner);
        drop(inner);
        if !pause.is_zero() {
            std::thread::sleep(pause);
        }
    }
}

impl Worker {
    /// `Err`: the server didn't answer; `Ok(None)`: it has nothing there.
    fn get(&self, inner: &Inner, path: &str) -> Result<Option<Vec<u8>>, ()> {
        use std::io::Read;
        let base = inner.base.get().ok_or(())?;
        match self.agent.get(&format!("{base}/{path}")).call() {
            Ok(response) if response.status() == 204 => Ok(None),
            Ok(response) => {
                let mut body = Vec::new();
                response
                    .into_reader()
                    .take(8 << 20)
                    .read_to_end(&mut body)
                    .map_err(|_| ())?;
                Ok(Some(body))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(_) => Err(()),
        }
    }
    fn failed(&mut self, inner: &Inner) {
        let mut state = inner.state.lock().unwrap();
        state.offline_until = Some(Instant::now() + self.backoff);
        // What waited for the server won't come: it is drawn without, and
        // wanted again once the server is given another try (`step`).
        for slot in state.tiles.values_mut() {
            if matches!(slot, Slot::Waiting) {
                *slot = Slot::Unreached;
            }
        }
        state.queue.clear();
        let base = inner.base.get().map_or("", String::as_str);
        log::debug!("Horizon: BlueMap at {base} isn't answering");
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
    }
    fn answered(&mut self) {
        self.backoff = BACKOFF;
    }
    /// One request, or a wait for one; how long to pause after it.
    fn step(&mut self, inner: &Inner) -> Duration {
        {
            let mut state = inner.state.lock().unwrap();
            if let Some(until) = state.offline_until {
                if let Some(wait) = until.checked_duration_since(Instant::now()) {
                    return wait.min(Duration::from_secs(1));
                }
                // Another try: what lookups found missing meanwhile is wanted.
                state.offline_until = None;
                state.reach(inner.radius, &inner.wake);
            }
        }
        if inner.base.get().is_none() {
            return self.look(inner);
        }
        let started = Instant::now();
        // The settings, once a visit: the cached ones serve until then.
        if !self.settings {
            let mut files = Vec::new();
            let mut unanswered = false;
            let map = inner.map_info(|path, name| {
                let body = self.get(inner, path).unwrap_or_else(|()| {
                    unanswered = true;
                    None
                })?;
                files.push((name.to_owned(), body.clone()));
                Some(body)
            });
            let Some(map) = map else {
                // The server answered, with no map of this level: nothing to
                // wait for on this visit.
                if !unanswered {
                    self.answered();
                    return self.found_nothing(inner);
                }
                self.failed(inner);
                return Duration::ZERO;
            };
            let (changed, stale) = {
                let state = inner.state.lock().unwrap();
                let old = state.map.as_deref();
                (
                    old != Some(&map),
                    old.is_some_and(|old| old != &map && old.id == map.id),
                )
            };
            // Tiles cut for other lowres settings would be read wrong: gone
            // before the new settings are kept, or they'd pass for theirs.
            let tiles = inner
                .dir
                .join(crate::server_terrain::server_folder(&map.id));
            self.settings = true;
            self.answered();
            // Not gone (a file held open): no map this visit, not the wrong one.
            if stale && std::fs::remove_dir_all(&tiles).is_err() && tiles.exists() {
                return self.found_nothing(inner);
            }
            for (name, body) in files {
                let _ = std::fs::create_dir_all(&inner.dir);
                // Not kept (a file held open): none cached rather than the
                // old, which would read the new tiles wrong next time.
                if write_whole(&inner.dir.join(&name), &body).is_err() {
                    let _ = std::fs::remove_file(inner.dir.join(name));
                }
            }
            let mut state = inner.state.lock().unwrap();
            if changed {
                state.map = Some(Arc::new(map));
                state.tiles.clear();
                state.chunks.clear();
                state.queue.clear();
                state.generation += 1;
                state.announce_all();
            }
            return REQUEST_GAP;
        }
        let next = {
            let mut state = inner.state.lock().unwrap();
            let picked = state.map.clone().and_then(|map| {
                let mut far = Vec::new();
                let mut best = None;
                for &key in &state.queue {
                    match state.near(&map.lowres, key, inner.radius) {
                        Some(d) if best.is_none_or(|(b, _)| d < b) => best = Some((d, key)),
                        Some(_) => {}
                        None => far.push(key),
                    }
                }
                // Out of reach since: forgotten, wanted again if it comes back.
                for key in far {
                    state.queue.remove(&key);
                    if let Some(slot @ Slot::Waiting) = state.tiles.get_mut(&key) {
                        *slot = Slot::Unreached;
                    }
                }
                let (_, key) = best?;
                state.queue.remove(&key);
                Some((map, key))
            });
            if picked.is_none() {
                let _ = inner.wake.wait_timeout(state, Duration::from_secs(1));
            }
            picked
        };
        let Some((map, key)) = next else {
            return Duration::ZERO;
        };
        let (level, x, z) = key;
        let path = format!(
            "{}/{}/tiles/{level}/{}.png",
            map.root,
            map.id,
            tile_path(x, z)
        );
        let body = match self.get(inner, &path) {
            Ok(body) => body,
            Err(()) => {
                self.failed(inner);
                return Duration::ZERO;
            }
        };
        self.answered();
        let tile = body.as_deref().and_then(|b| decode_tile(b, &map.lowres));
        let file = inner.tile_file(&map, key);
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&inner.dir));
        let (png, none) = (file.with_extension("png"), file.with_extension("none"));
        // Whether what is cached (and drawn) stays as it was, and the bytes
        // written (`None` also when a write failed: the new tile is still
        // drawn this visit).
        let (unchanged, written) = match (&tile, &body) {
            (Some(_), Some(body)) if std::fs::read(&png).is_ok_and(|old| old == *body) => {
                // Unchanged: fresh for another `MAX_AGE`.
                let _ = std::fs::File::options()
                    .append(true)
                    .open(&png)
                    .and_then(|f| f.set_modified(SystemTime::now()));
                (true, None)
            }
            (Some(_), Some(body)) => {
                let _ = std::fs::remove_file(&none);
                (
                    false,
                    write_whole(&png, body).ok().map(|()| body.len() as u64),
                )
            }
            // Not a tile (an error page): a cached one is kept.
            (None, Some(_)) if png.exists() => (true, None),
            _ => {
                let _ = std::fs::remove_file(&png);
                (false, write_whole(&none, &[]).ok().map(|()| 0))
            }
        };
        // The disk walks outside the lock: lookups and the frame wait on it.
        if let Some(bytes) = written {
            let disk = self.disk.get_or_insert_with(|| folder_size(&inner.dir));
            *disk += bytes;
            if *disk > DISK_CAP {
                *disk = prune(&inner.dir, DISK_CAP * 3 / 4);
            }
        }
        let mut state = inner.state.lock().unwrap();
        let waited = matches!(state.tiles.get(&key), Some(Slot::Waiting));
        // Unchanged and drawn already: nothing to tell.
        if unchanged && !waited {
            return REQUEST_GAP.saturating_sub(started.elapsed());
        }
        let slot = tile.map_or(Slot::Missing, |t| Slot::Ready(Arc::new(t)));
        state.tiles.insert(key, slot);
        let (x0, x1) = map.lowres.span(0, level, x);
        let (z0, z1) = map.lowres.span(1, level, z);
        // Only level 1 makes chunks; their depth reads a pixel around.
        state.generation += 1;
        if level == 1 {
            state.chunks.retain(|&(cx, cz), _| {
                cx * 16 > x1 || (cx + 1) * 16 < x0 || cz * 16 > z1 || (cz + 1) * 16 < z0
            });
        }
        state.announce(Change {
            area: [x0, z0, x1, z1],
            widths: map.lowres.widths(level),
        });
        REQUEST_GAP.saturating_sub(started.elapsed())
    }
}

impl Worker {
    /// One step of looking for the map: the next candidate, one at a time
    /// and one look a server at a time (another level's map may be looking).
    fn look(&mut self, inner: &Inner) -> Duration {
        use crate::bluemap_detect::{self as detect, Claim, Remembered};
        if self.claim.is_none() {
            let Some(claim) = Claim::take(&inner.dir) else {
                return Duration::from_secs(1);
            };
            self.claim = Some(claim);
            // What a look finished meanwhile found: one of this server's
            // addresses only (`bluemap_detect::source`).
            match detect::remembered(&inner.dir, SystemTime::now()) {
                Remembered::Found(url) if !inner.probes.contains(&url) => {}
                Remembered::Found(url) => {
                    let _ = inner.base.set(url);
                    self.claim = None;
                    return Duration::ZERO;
                }
                Remembered::Nothing => return self.found_nothing(inner),
                Remembered::Stale(_) => {}
            }
        }
        let Some(candidate) = inner.probes.get(self.probed) else {
            detect::remember(&inner.dir, None, SystemTime::now());
            log::debug!("Horizon: no BlueMap found for {}", inner.level);
            return self.found_nothing(inner);
        };
        self.probed += 1;
        let started = Instant::now();
        if detect::probe(&self.agent, candidate) {
            log::info!("Horizon: found the server's BlueMap");
            log::debug!("Horizon: BlueMap at {candidate}");
            detect::remember(&inner.dir, Some(candidate), SystemTime::now());
            let _ = inner.base.set(candidate.clone());
            self.claim = None;
            return Duration::ZERO;
        }
        log::debug!("Horizon: no BlueMap at {candidate}");
        detect::GAP.saturating_sub(started.elapsed())
    }
    /// The server has no map: what was cached of an earlier one isn't drawn.
    fn found_nothing(&mut self, inner: &Inner) -> Duration {
        let mut state = inner.state.lock().unwrap();
        if state.map.take().is_some() {
            state.tiles.clear();
            state.chunks.clear();
            state.generation += 1;
            state.announce_all();
        }
        self.claim = None;
        self.done = true;
        Duration::ZERO
    }
}

fn cached_files(dir: &Path, out: &mut Vec<(SystemTime, u64, PathBuf)>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            cached_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "png" || e == "none") {
            out.push((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                meta.len(),
                path,
            ));
        }
    }
}
fn folder_size(dir: &Path) -> u64 {
    let mut files = Vec::new();
    cached_files(dir, &mut files);
    files.iter().map(|(_, len, _)| len).sum()
}
/// Deletes the oldest tiles until at most `keep` bytes are left; the bytes left.
fn prune(dir: &Path, keep: u64) -> u64 {
    let mut files = Vec::new();
    cached_files(dir, &mut files);
    files.sort_unstable();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    for (_, len, path) in files {
        if total <= keep {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOWRES: Lowres = Lowres {
        tile: [32, 32],
        factor: 5,
        levels: 3,
    };

    /// A lowres tile as BlueMap writes it: `pixel` gives each column.
    fn tile_png(lowres: &Lowres, pixel: impl Fn(usize, usize) -> Pixel) -> Vec<u8> {
        let (w, h) = (lowres.tile[0] as usize + 1, lowres.tile[1] as usize + 1);
        let mut data = vec![0u8; w * h * 2 * 4];
        for z in 0..h {
            for x in 0..w {
                let p = pixel(x, z);
                let [a, r, g, b] = p.color.to_be_bytes();
                let [hi, lo] = p.height.to_be_bytes();
                data[(z * w + x) * 4..][..4].copy_from_slice(&[r, g, b, a]);
                data[((z + h) * w + x) * 4..][..4].copy_from_slice(&[p.block_light, hi, lo, 255]);
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w as u32, (h * 2) as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        out
    }
    fn sample(x: usize, z: usize) -> Pixel {
        Pixel {
            color: 0xFF00_0000 | (x as u32) << 16 | (z as u32) << 8 | 0x40,
            height: x as i16 * 3 - 60,
            block_light: (z % 16) as u8,
        }
    }

    #[test]
    fn tiles_decode_colour_height_and_light() {
        let png = tile_png(&LOWRES, sample);
        let tile = decode_tile(&png, &LOWRES).expect("tile");
        for (x, z) in [(0, 0), (5, 31), (32, 32), (20, 3)] {
            assert_eq!(tile.pixel(x, z), sample(x, z));
        }
        assert_eq!(tile.pixel(0, 0).height, -60);
        // Gzipped by a web server, still the same tile.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &png).unwrap();
        assert_eq!(decode_tile(&gz.finish().unwrap(), &LOWRES), Some(tile));
        // Another tile size is refused.
        let small = Lowres {
            tile: [16, 16],
            ..LOWRES
        };
        assert_eq!(decode_tile(&png, &small), None);
        assert_eq!(decode_tile(b"not a png", &LOWRES), None);
    }

    #[test]
    fn tile_paths_split_digits_as_the_web_app() {
        assert_eq!(tile_path(0, 0), "x0/z0");
        assert_eq!(tile_path(12, -3), "x1/2/z-3");
        assert_eq!(tile_path(-120, 45), "x-1/2/0/z4/5");
    }

    #[test]
    fn blocks_find_their_tile_and_pixel_at_every_level() {
        // Level 1: a pixel a block.
        assert_eq!(LOWRES.locate(0, 1, 0), (0, 0));
        assert_eq!(LOWRES.locate(0, 1, 33), (1, 1));
        assert_eq!(LOWRES.locate(0, 1, -1), (-1, 31));
        // Level 2: five level-1 tiles a tile, six pixels of five blocks each.
        assert_eq!(LOWRES.locate(0, 2, 0), (0, 0));
        assert_eq!(LOWRES.locate(0, 2, 4), (0, 0));
        assert_eq!(LOWRES.locate(0, 2, 5), (0, 1));
        assert_eq!(LOWRES.locate(0, 2, 32 + 7), (0, 7));
        assert_eq!(
            LOWRES.locate(0, 2, 31),
            (0, 5),
            "the leftover blocks join the last pixel"
        );
        assert_eq!(LOWRES.locate(0, 2, -1), (-1, 29));
        assert_eq!(LOWRES.locate(0, 2, 5 * 32), (1, 0));
        // Level 3: level 2's tiles packed the same way.
        assert_eq!(LOWRES.locate(0, 3, 5 * 32 * 2 + 10), (0, 12));
        assert_eq!(LOWRES.locate(0, 3, -1), (-1, 29));
        // Every block of a level-3 tile's span lands in that tile.
        let (min, max) = LOWRES.span(0, 3, -2);
        assert_eq!((min, max), (-1600, -800));
        assert!(
            (min..max)
                .step_by(7)
                .all(|b| LOWRES.locate(0, 3, b).0 == -2)
        );
        assert_eq!(LOWRES.level_for(1), 1);
        assert_eq!(LOWRES.level_for(16), 2);
        assert_eq!(LOWRES.level_for(1024), 3);
    }

    #[test]
    fn settings_files_parse() {
        let root = Root::parse(br#"{"version":"5.4","maps":["world","world_nether","world_the_end"],"mapDataRoot":"maps/","liveDataRoot":"maps"}"#).unwrap();
        assert_eq!(root.data_root, "maps");
        let map = br#"{"name":"World","hires":{"tileSize":[32,32]},"lowres":{"tileSize":[50,40],"lodFactor":4,"lodCount":2}}"#;
        assert_eq!(
            Lowres::parse(map),
            Some(Lowres {
                tile: [50, 40],
                factor: 4,
                levels: 2
            })
        );
        assert_eq!(Lowres::parse(br#"{"lowres":{}}"#), Some(LOWRES));
        assert_eq!(Lowres::parse(br#"{"lowres":{"lodFactor":0}}"#), None);
        assert_eq!(
            pick_map(&root.maps, "minecraft:overworld", &[]).as_deref(),
            Some("world")
        );
        assert_eq!(
            pick_map(&root.maps, "minecraft:the_nether", &[]).as_deref(),
            Some("world_nether")
        );
        assert_eq!(
            pick_map(&root.maps, "minecraft:the_end", &[]).as_deref(),
            Some("world_the_end")
        );
        let chosen = [("minecraft:overworld".to_owned(), "survival".to_owned())];
        assert_eq!(
            pick_map(&root.maps, "minecraft:overworld", &chosen).as_deref(),
            Some("survival")
        );
        assert_eq!(pick_map(&root.maps, "mod:moon", &[]), None);
        assert_eq!(
            base_url(" https://map.example.net/#world:1:2:3 "),
            "https://map.example.net"
        );
        assert_eq!(
            base_url("http://example.net:8100/index.html"),
            "http://example.net:8100"
        );
    }

    #[test]
    fn player_settings_round_trip() {
        let server = Settings::parse_server(
            "play.example.net:25566 https://map.example.net nether=world_nether mod:moon=moon",
        )
        .unwrap();
        assert_eq!(
            server.maps,
            [
                ("minecraft:the_nether".to_owned(), "world_nether".to_owned()),
                ("mod:moon".to_owned(), "moon".to_owned())
            ]
        );
        let settings = Settings {
            radius: 32,
            auto_detect: true,
            servers: vec![server.clone()],
        };
        let text = settings.serialize();
        let mut parsed = Settings::default();
        for line in text.lines() {
            match line.split_once(':') {
                Some(("blueMapRadius", v)) => parsed.radius = Settings::parse_radius(v).unwrap(),
                Some(("blueMapAutoDetect", v)) => parsed.auto_detect = v == "true",
                _ => panic!("{line}: no map in the graphics settings"),
            }
        }
        parsed.servers = Settings::parse_servers(
            "\u{feff}# a comment\r\nblueMap: play.example.net:25566 https://map.example.net nether=world_nether mod:moon=moon\nlodDistance:64\n",
        );
        assert_eq!(parsed, settings);
        assert!(settings.server("Play.Example.net:25566").is_some());
        assert!(settings.server("play.example.net").is_none());
        assert_eq!(Settings::parse_radius("48"), Some(48));
        assert_eq!(Settings::parse_radius("2000b"), Some(125));
        assert_eq!(Settings::parse_radius("1024 blocks"), Some(64));
        assert_eq!(Settings::parse_radius("lots"), None);
        assert_eq!(
            Settings::default().radius_blocks(),
            DEFAULT_RADIUS as i32 * 16
        );
    }

    fn palette() -> Palette {
        let names = [
            "stone",
            "dirt",
            "sand",
            "grass_block",
            "oak_leaves",
            "water",
            "snow_block",
        ];
        Palette::new(
            |name| {
                let name = name.strip_prefix("minecraft:")?;
                names
                    .iter()
                    .position(|n| *n == name)
                    .map(|i| BlockStateId(i as u32 + 1))
            },
            &[
                Tints {
                    id: 0,
                    grass: 0x91BD59,
                    foliage: 0x77AB2F,
                    water: 0x3F76E4,
                },
                Tints {
                    id: 1,
                    grass: 0xBFB755,
                    foliage: 0xAEA42A,
                    water: 0x3F76E4,
                },
            ],
            0,
        )
    }

    #[test]
    fn colours_become_the_nearest_block() {
        let palette = palette();
        let at = |color: u32, height: i16| {
            let pixel = Pixel {
                color: 0xFF00_0000 | color,
                height,
                block_light: 0,
            };
            palette.column(pixel, i32::from(height) + 1)
        };
        let grass = at(0x577133, 70);
        let surface = grass.surface.unwrap();
        assert_eq!(
            (surface.height, surface.state, surface.biome),
            (71, BlockStateId(4), 0)
        );
        assert_eq!(grass.spans.last().unwrap().max, 71);
        assert_eq!(
            grass.spans[grass.spans.len() - 2].state,
            BlockStateId(2),
            "dirt under grass"
        );
        // As deep as its neighbours need, not down to the void.
        assert_eq!(grass.spans[0].min, 71 - SKIRT as i16);
        let cliff = palette.column(
            Pixel {
                color: 0xFF57_7133,
                height: 70,
                block_light: 0,
            },
            40,
        );
        assert_eq!(cliff.spans[0].min, 40 - SKIRT as i16);
        // A savanna-ish green picks that biome's tint.
        assert_eq!(at(0x736E33, 70).surface.unwrap().biome, 1);
        let sea = at(0x2C52A0, 62).surface.unwrap();
        assert_eq!(sea.water, Some((63, BlockStateId(6))));
        assert_eq!(sea.state, BlockStateId(3));
        assert_eq!(at(0xF0F5F5, 90).surface.unwrap().state, BlockStateId(7));
        assert_eq!(at(0x7A7A7A, -10).surface.unwrap().state, BlockStateId(1));
        // Nothing rendered there: no column.
        assert_eq!(
            palette.column(
                Pixel {
                    color: 0x00FF_FFFF,
                    height: 64,
                    block_light: 0
                },
                65
            ),
            Column::default()
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-bluemap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Cached settings and tiles serve without the server; a missing tile
    /// is wanted only near the player, and real data waits for it.
    #[test]
    fn cached_tiles_serve_and_far_ones_are_not_wanted() {
        let dir = scratch("cache");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), br#"{"maps":["world"]}"#).unwrap();
        std::fs::write(dir.join("world.json"), br#"{"lowres":{}}"#).unwrap();
        let green = |_: usize, _: usize| Pixel {
            color: 0xFF57_7133,
            height: 70,
            block_light: 0,
        };
        let tiles = dir.join("world/1");
        std::fs::create_dir_all(&tiles).unwrap();
        std::fs::write(tiles.join("0_-1.png"), tile_png(&LOWRES, green)).unwrap();
        std::fs::write(tiles.join("1_-1.none"), []).unwrap();
        let map = BlueMap::new(
            "https://map.example.net/",
            "minecraft:overworld",
            Vec::new(),
            dir.clone(),
            256,
            palette(),
        );
        // Chunk (1, -1) lies in tile (0, -1).
        let Found::Ready(columns) = map.chunk(1, -1) else {
            panic!("cached tile")
        };
        assert_eq!(columns[0].surface.map(|s| s.height), Some(71));
        assert_eq!(
            map.chunk(2, -1),
            Found::Absent,
            "the server has nothing there"
        );
        // No player yet: nothing is near, nothing waited for.
        assert_eq!(map.chunk(40, 40), Found::Absent);
        let queued = |key| map.inner.state.lock().unwrap().queue.contains(&key);
        // The player comes: what was looked up in reach is wanted, and far
        // terrain has nothing to sample again until it arrives.
        map.set_center(40 * 16, 40 * 16);
        assert!(map.take_changed().is_empty());
        assert_eq!(map.chunk(40, 40), Found::Waiting);
        assert!(queued((1, 20, 20)));
        assert_eq!(map.chunk(-400, 0), Found::Absent, "past the radius");
        // A wide cell reads the coarsest level, here not cached either.
        assert_eq!(map.reduced_column(40 * 16, 40 * 16, 1024), Found::Waiting);
        assert!(queued((3, 1, 1)));
        // Out of reach, not looked up on disk again; wanted once in reach.
        assert_eq!(map.chunk(60, 40), Found::Absent);
        assert!(matches!(
            map.inner.state.lock().unwrap().tiles.get(&(1, 30, 20)),
            Some(Slot::Unreached)
        ));
        map.set_center(40 * 16 + 10, 40 * 16);
        assert!(!queued((1, 30, 20)), "moving a little wants nothing");
        map.set_center(40 * 16 + 100, 40 * 16);
        assert!(queued((1, 30, 20)));
        assert!(map.take_changed().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Real chunks the server sent come first; BlueMap fills the rest.
    #[test]
    fn stored_chunks_win_over_the_map() {
        use crate::{server_terrain::ServerTerrain, streaming::LodSource};
        let dir = scratch("terrain");
        let maps = dir.join("map");
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::write(maps.join("settings.json"), br#"{"maps":["world"]}"#).unwrap();
        std::fs::write(maps.join("world.json"), br#"{"lowres":{}}"#).unwrap();
        let green = |_: usize, _: usize| Pixel {
            color: 0xFF57_7133,
            height: 70,
            block_light: 0,
        };
        for level in 1..=3 {
            let tiles = maps.join(format!("world/{level}"));
            std::fs::create_dir_all(&tiles).unwrap();
            std::fs::write(tiles.join("0_0.png"), tile_png(&LOWRES, green)).unwrap();
        }
        let map = BlueMap::new(
            "http://map.example.net",
            "minecraft:overworld",
            Vec::new(),
            maps,
            256,
            palette(),
        );
        let terrain = ServerTerrain::new(dir.join("chunks"), Some(Arc::new(map)));
        let real = Column::from_surface(Some(crate::lod::Surface {
            height: 100,
            state: BlockStateId(9),
            water: None,
            biome: 0,
        }));
        let columns = Arc::new(std::array::from_fn(|_| real.clone()));
        terrain.store((0, 0), &columns).unwrap();
        let mut sampler = terrain.clone().sampler();
        assert_eq!(sampler.sample(3, 3).map(|s| s.height), Some(100));
        assert_eq!(sampler.sample(20, 3).map(|s| s.height), Some(71));
        assert_eq!(sampler.sample(-1, 3), None, "neither has it");
        // Wider cells are reduced to spans: their top.
        let mut top = |x, z, width| {
            let column = sampler.reduced_column(x, z, width);
            column.spans.last().map(|s| s.max)
        };
        assert_eq!(top(16, 0, 8), Some(71));
        assert_eq!(top(0, 0, 16), Some(100));
        // A wide cell: the stored chunk at its middle, else the map.
        assert_eq!(top(0, 0, 64), Some(71));
        assert_eq!(top(-16, -16, 32), Some(100));
        sampler.request_detail(&[(0, 0), (1, 0), (-1, 0)], [0, 0]);
        let details = sampler.drain_details();
        assert_eq!(details.len(), 3);
        assert_eq!(details[0].1, columns);
        assert_eq!(details[1].1[0].surface.map(|s| s.height), Some(71));
        assert_eq!(details[2].1[0], Column::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arrivals_name_the_cells_they_change_and_come_in_bursts() {
        // Cells up to a chunk read level 1, wider ones level 3 (level 2's
        // 5-block pixels suit no power-of-two cell).
        assert_eq!(LOWRES.widths(1), (1, 16));
        assert_eq!(LOWRES.widths(2), (17, 24));
        assert_eq!(LOWRES.widths(3), (25, i32::MAX));
        let map = BlueMap::new(
            "http://map.example.net",
            "minecraft:overworld",
            Vec::new(),
            scratch("bursts"),
            256,
            palette(),
        );
        let change = |x| Change {
            area: [x, 0, x + 32, 32],
            widths: LOWRES.widths(1),
        };
        let announce = |x| map.inner.state.lock().unwrap().announce(change(x));
        announce(0);
        assert_eq!(map.take_changed(), [change(0)]);
        announce(32);
        announce(64);
        assert!(map.take_changed().is_empty(), "within CHANGE_GAP");
        map.inner.state.lock().unwrap().taken_at = Some(Instant::now() - CHANGE_GAP);
        assert_eq!(map.take_changed(), [change(32), change(64)]);
    }

    /// A chunk's columns reach under their neighbours' tops, the ones
    /// beside the map's edge no further than their own.
    #[test]
    fn chunks_are_as_deep_as_their_neighbours() {
        let dir = scratch("depth");
        std::fs::create_dir_all(dir.join("world/1")).unwrap();
        std::fs::write(dir.join("settings.json"), br#"{"maps":["world"]}"#).unwrap();
        std::fs::write(dir.join("world.json"), br#"{"lowres":{}}"#).unwrap();
        // A step down at x = 8, and nothing rendered from x = 24.
        let step = |x: usize, _: usize| Pixel {
            color: if x >= 24 { 0 } else { 0xFF57_7133 },
            height: if x < 8 { 90 } else { 70 },
            block_light: 0,
        };
        std::fs::write(dir.join("world/1/0_0.png"), tile_png(&LOWRES, step)).unwrap();
        let map = BlueMap::new(
            "http://map.example.net",
            "minecraft:overworld",
            Vec::new(),
            dir.clone(),
            256,
            palette(),
        );
        let Found::Ready(left) = map.chunk(0, 0) else {
            panic!("cached tile")
        };
        let Found::Ready(right) = map.chunk(1, 0) else {
            panic!("cached tile")
        };
        let bottom = |c: &Column| c.spans[0].min;
        assert_eq!(bottom(&left[16 + 7]), 71 - SKIRT as i16, "the cliff's top");
        assert_eq!(bottom(&left[16 + 3]), 91 - SKIRT as i16);
        assert_eq!(
            bottom(&right[16 + 7]),
            71 - SKIRT as i16,
            "by the map's edge"
        );
        assert_eq!(right[16 + 8], Column::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_oldest_tiles_go_past_the_cap() {
        let dir = scratch("prune");
        std::fs::create_dir_all(dir.join("a/1")).unwrap();
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            let path = dir.join(format!("a/1/{name}.png"));
            std::fs::write(&path, [0u8; 100]).unwrap();
            let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + i as u64);
            std::fs::File::options()
                .append(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        std::fs::write(dir.join("settings.json"), [0u8; 50]).unwrap();
        assert_eq!(folder_size(&dir), 300);
        assert_eq!(prune(&dir, 150), 100);
        assert!(dir.join("a/1/new.png").exists() && !dir.join("a/1/mid.png").exists());
        assert!(dir.join("settings.json").exists(), "only tiles are pruned");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
