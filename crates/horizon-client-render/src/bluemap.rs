//! Far terrain from a server's BlueMap: the
//! lowres tiles of the server's web map, fetched over HTTP around the player,
//! fill the far terrain where no real chunk was ever seen
//! (`server_terrain`'s store always comes first). Off unless the
//! player names the server's map (`Settings`).
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
use std::sync::Mutex;

/// Where the player names the servers' maps (`Settings::servers`), beside
/// `horizon-graphics.txt`: kept out of the graphics settings, which shared
/// instances and modpacks carry, as a map the client asks for tiles around
/// the player (where they stand, which server) is the player's to name, not
/// a pack's.
pub const SERVERS_FILE: &str = "horizon-bluemap.txt";
/// The player's BlueMap settings: `blueMapRadius:<chunks>` (or `<blocks>b`)
/// in `horizon-graphics.txt`, and
/// `blueMap:<server address> <map URL> [<level>=<map id> ...]` per server in
/// `SERVERS_FILE`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// How far around the player tiles are downloaded, in chunks; 0 uses
    /// `DEFAULT_RADIUS`.
    pub radius: u32,
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
        format!("blueMapRadius:{}\n", self.radius)
    }
    /// The settings of the server at `address`, as typed to join it.
    pub fn server(&self, address: &str) -> Option<&Server> {
        let folder = crate::server_terrain::server_folder(address);
        self.servers
            .iter()
            .find(|s| crate::server_terrain::server_folder(&s.address) == folder)
    }
    pub fn radius_blocks(&self) -> i32 {
        let chunks = if self.radius == 0 {
            DEFAULT_RADIUS
        } else {
            self.radius
        };
        chunks as i32 * 16
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
    fn nearest(&self, color: u32) -> Option<Candidate> {
        let key = color & 0xFF_FFFF;
        let mut matches = self.matches.lock().unwrap();
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
    /// blocks under it, then stone; water over sand.
    pub fn column(&self, pixel: Pixel) -> Column {
        if pixel.color >> 24 == 0 {
            return Column::default();
        }
        let Some(c) = self.nearest(pixel.color) else {
            return Column::default();
        };
        let top = i32::from(pixel.height) + 1;
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
            push(MIN_Y, floor - 3, self.deep);
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
            push(MIN_Y, top - 4, self.deep);
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

/// `https://map.example.net/#world:0:64:0...` → `https://map.example.net`.
pub fn base_url(url: &str) -> String {
    let url = url.trim();
    let url = url.split(['#', '?']).next().unwrap_or(url);
    let url = url.strip_suffix("index.html").unwrap_or(url);
    url.trim_end_matches('/').to_owned()
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
            servers: vec![server.clone()],
        };
        let text = settings.serialize();
        let mut parsed = Settings::default();
        for line in text.lines() {
            match line.split_once(':') {
                Some(("blueMapRadius", v)) => parsed.radius = Settings::parse_radius(v).unwrap(),
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
            palette.column(Pixel {
                color: 0xFF00_0000 | color,
                height,
                block_light: 0,
            })
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
        // A savanna-ish green picks that biome's tint.
        assert_eq!(at(0x736E33, 70).surface.unwrap().biome, 1);
        let sea = at(0x2C52A0, 62).surface.unwrap();
        assert_eq!(sea.water, Some((63, BlockStateId(6))));
        assert_eq!(sea.state, BlockStateId(3));
        assert_eq!(at(0xF0F5F5, 90).surface.unwrap().state, BlockStateId(7));
        assert_eq!(at(0x7A7A7A, -10).surface.unwrap().state, BlockStateId(1));
        // Nothing rendered there: no column.
        assert_eq!(
            palette.column(Pixel {
                color: 0x00FF_FFFF,
                height: 64,
                block_light: 0
            }),
            Column::default()
        );
    }
}
