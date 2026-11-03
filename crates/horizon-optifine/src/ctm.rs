//! Connected textures (`optifine/ctm/**/*.properties`): the rules a pack defines, which block
//! faces each one applies to, and its replacement tiles. Choosing a tile from the neighbours is
//! [`connect`]'s.
//!
//! Written from OptiFine's `ctm.properties` documentation and template images, and Continuity's
//! wiki (its "Continuity Connected Textures Specification" page) for what OptiFine does where the
//! documentation is silent; no mod's source was read.

pub mod connect;

use crate::{BlockMatch, IdList, Name, Properties, RangeList};
use std::path::Path;

/// In this order: the overlays last ([`Method::is_overlay`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    /// The 47 tiles of the 8-way template.
    Ctm,
    /// 5 tiles, cut into quarters, and `ctm.N` replacements.
    CtmCompact,
    Horizontal,
    Vertical,
    HorizontalVertical,
    VerticalHorizontal,
    Top,
    Random,
    Repeat,
    Fixed,
    /// The 17 transition tiles drawn over a face beside the connecting blocks.
    Overlay,
    OverlayCtm,
    OverlayRandom,
    OverlayRepeat,
    OverlayFixed,
    /// Continuity's overlay forms of the 4-tile methods.
    OverlayHorizontal,
    OverlayVertical,
    OverlayHorizontalVertical,
    OverlayVerticalHorizontal,
}

impl Method {
    fn parse(value: &str) -> Option<Self> {
        use Method::*;
        Some(match value {
            // MCPatcher's names for the same methods.
            "ctm" | "glass" => Ctm,
            "ctm_compact" => CtmCompact,
            "horizontal" | "bookshelf" => Horizontal,
            "vertical" => Vertical,
            "horizontal+vertical" | "h+v" => HorizontalVertical,
            "vertical+horizontal" | "v+h" => VerticalHorizontal,
            "top" | "sandstone" => Top,
            "random" => Random,
            "repeat" | "pattern" => Repeat,
            "fixed" | "static" => Fixed,
            "overlay" => Overlay,
            "overlay_ctm" => OverlayCtm,
            "overlay_random" => OverlayRandom,
            "overlay_repeat" => OverlayRepeat,
            "overlay_fixed" => OverlayFixed,
            "overlay_horizontal" => OverlayHorizontal,
            "overlay_vertical" => OverlayVertical,
            "overlay_horizontal+vertical" | "overlay_h+v" => OverlayHorizontalVertical,
            "overlay_vertical+horizontal" | "overlay_v+h" => OverlayVerticalHorizontal,
            _ => return None,
        })
    }

    /// Draws a second quad over the face instead of replacing its sprite.
    pub fn is_overlay(self) -> bool {
        self >= Method::Overlay
    }

    /// The fewest tiles the method takes, and the exact count when it needs one. `ctm_compact`
    /// takes extra tiles for its `ctm.N` replacements; `repeat` needs `width * height`.
    fn tiles_needed(self, width: u32, height: u32) -> (usize, Option<usize>) {
        use Method::*;
        let n = |n| (n, Some(n));
        match self {
            Ctm | OverlayCtm => n(47),
            CtmCompact => (5, None),
            Horizontal | Vertical | OverlayHorizontal | OverlayVertical => n(4),
            HorizontalVertical
            | VerticalHorizontal
            | OverlayHorizontalVertical
            | OverlayVerticalHorizontal => n(7),
            Top | Fixed | OverlayFixed => n(1),
            Random | OverlayRandom => (1, None),
            // `checked_mul`'s `None` (a pattern no pack can fill) asks for 0 tiles: refused.
            Repeat | OverlayRepeat => n(width.checked_mul(height).unwrap_or(0) as usize),
            Overlay => n(17),
        }
    }
}

/// When two neighbouring faces join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connect {
    /// The same block.
    Block,
    /// The same sprite on that face.
    Tile,
    /// The same block state.
    State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tile {
    /// A texture's resource path (`assets/<ns>/....png`).
    Texture(String),
    /// Nothing from this rule: the next rule is tried (an overlay draws nothing).
    Skip,
    /// The face's own sprite.
    Default,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Symmetry {
    #[default]
    None,
    /// Opposite faces share a choice.
    Opposite,
    /// All six faces share a choice.
    All,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    #[default]
    Cutout,
    Translucent,
}

/// How a face's left, right, up and down are found (Continuity's `orient`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orient {
    /// The face's directions on a vanilla cube.
    None,
    /// [`Orient::None`], turned with the block's `axis` as a cube column's model is.
    StateAxis,
    /// The quad's own texture directions.
    Texture,
}

/// Bits of [`Rule::faces`], by `Direction` order: down, up, north, south, west, east.
pub mod faces {
    pub const DOWN: u8 = 1;
    pub const UP: u8 = 2;
    pub const NORTH: u8 = 4;
    pub const SOUTH: u8 = 8;
    pub const WEST: u8 = 16;
    pub const EAST: u8 = 32;
    pub const SIDES: u8 = NORTH | SOUTH | WEST | EAST;
    pub const ALL: u8 = 63;
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// The properties file's resource path: rules are ordered by it.
    pub path: String,
    pub method: Method,
    pub tiles: Vec<Tile>,
    /// Sprites (as texture resource paths) whose faces the rule replaces.
    pub match_tiles: Vec<String>,
    pub match_blocks: Vec<BlockMatch>,
    pub weight: i32,
    /// [`faces`] bits.
    pub faces: u8,
    pub biomes: Option<IdList>,
    pub heights: Option<RangeList>,
    /// A nameable block entity's custom name.
    pub name: Option<Name>,
    pub connect: Connect,
    /// Overlays: join only to faces with these sprites (texture resource paths) ...
    pub connect_tiles: Vec<String>,
    /// ... or these blocks.
    pub connect_blocks: Vec<BlockMatch>,
    pub inner_seams: bool,
    /// One per tile (`random`); empty when every tile counts the same.
    pub weights: Vec<u32>,
    pub random_loops: u32,
    pub symmetry: Symmetry,
    pub linked: bool,
    /// `repeat`'s pattern size, in blocks.
    pub width: u32,
    pub height: u32,
    /// `ctm_compact`'s `ctm.<case>=<tile index>`, by 47-tile case.
    pub compact_overrides: Vec<(u8, usize)>,
    /// Overlays: the tint index and the block whose tint it is (`None`: the face's block).
    pub tint_index: i32,
    pub tint_block: Option<String>,
    pub layer: Layer,
    pub orient: Orient,
    /// Applied before every rule without it (Continuity's `prioritize`; OptiFine's "tile-based
    /// entries are checked before block-based ones").
    pub prioritize: bool,
}

/// Every rule `root` holds, in the order they are tried: prioritized (tile) rules first, then by
/// weight (highest first), then by path. Broken files are logged and left out.
pub fn load(root: &Path) -> Vec<Rule> {
    let mut rules: Vec<Rule> = crate::properties_files(root, "ctm")
        .into_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(root.join(&path))
                .map_err(|e| log::warn!("Horizon: connected textures {path}: {e}"))
                .ok()?;
            // Java's `Properties.load(InputStream)` reads ISO-8859-1; UTF-8 when it is valid.
            let text = String::from_utf8(bytes)
                .unwrap_or_else(|e| e.into_bytes().iter().map(|&b| char::from(b)).collect());
            Rule::parse(&path, &Properties::parse(&text))
                .map_err(|e| log::warn!("Horizon: connected textures {path}: {e}"))
                .ok()
                .flatten()
        })
        .collect();
    sort(&mut rules);
    rules
}

/// [`load`]'s order, for rules gathered elsewhere.
pub fn sort(rules: &mut [Rule]) {
    rules.sort_by(|a, b| (b.prioritize, b.weight, &a.path).cmp(&(a.prioritize, a.weight, &b.path)));
}

impl Rule {
    /// `Ok(None)` for a file the pack keeps from Continuity (`optifineOnly=true`).
    pub fn parse(path: &str, props: &Properties) -> Result<Option<Self>, String> {
        let get = |key| props.get(key).filter(|v| !v.is_empty());
        // `Boolean.parseBoolean`.
        let flag = |key| get(key).map(|v: &str| v.eq_ignore_ascii_case("true"));
        // Continuity's wiki: `optifineOnly` counts only in `minecraft`, the namespace OptiFine
        // reads, so a pack can keep a file from Continuity there and give it another elsewhere.
        let namespace = path.split('/').nth(1).unwrap_or("minecraft");
        if namespace == "minecraft" && flag("optifineOnly") == Some(true) {
            return Ok(None);
        }
        let method = get("method").ok_or("no method")?;
        let method = Method::parse(method).ok_or(format!("unknown method {method}"))?;

        let stem = path.rsplit('/').next().unwrap_or(path);
        let stem = stem.strip_suffix(".properties").unwrap_or(stem);
        let (mut match_tiles, mut match_blocks) = (Vec::new(), Vec::new());
        match (get("matchTiles"), get("matchBlocks")) {
            (None, None) => match stem.strip_prefix("block_") {
                // `block_<name>.properties` and `<name>.properties` name what they match.
                Some(block) => match_blocks = BlockMatch::parse_list(block),
                None => match_tiles.extend(sprite_path(stem)),
            },
            (tiles, blocks) => {
                match_tiles = tiles.map_or_else(Vec::new, sprite_list);
                match_blocks = blocks.map_or_else(Vec::new, BlockMatch::parse_list);
            }
        }
        if match_tiles.is_empty() && match_blocks.is_empty() {
            return Err("matches no block or tile".into());
        }

        let int = |key, default| get(key).and_then(|v| v.parse().ok()).unwrap_or(default);
        let width = int("width", 0).max(0) as u32;
        let height = int("height", 0).max(0) as u32;

        let tiles = match get("tiles") {
            Some(v) => tile_list(path, v)?,
            None => Vec::new(),
        };
        let (min, max) = method.tiles_needed(width, height);
        if tiles.len() < min || max.is_some_and(|max| tiles.len() != max) || min == 0 {
            return Err(format!("{} tiles for {method:?}", tiles.len()));
        }
        if method.is_overlay() && tiles.contains(&Tile::Default) {
            return Err("an overlay has no <default> tile".into());
        }

        let mut weights: Vec<u32> = get("weights")
            .map(|v| {
                // By position: a weight that is not a number (`Integer.parseInt`) counts 1.
                v.split_whitespace()
                    .map(|w| w.parse::<i32>().map_or(1, |w| w.max(0) as u32))
                    .collect()
            })
            .unwrap_or_default();
        if !weights.is_empty() {
            weights.resize(tiles.len(), 1);
        }
        if weights.iter().all(|&w| w == 0) {
            // Nothing to pick by: every tile alike.
            weights.clear();
        }

        let compact_overrides = props
            .with_prefix("ctm.")
            .filter_map(|(case, tile)| {
                let case: u8 = case.parse().ok().filter(|&c| c < 47)?;
                let tile: usize = tile.parse().ok().filter(|&t| t < tiles.len())?;
                Some((case, tile))
            })
            .collect();

        let heights = match (get("heights"), get("minHeight"), get("maxHeight")) {
            (Some(h), ..) => Some(height_list(h).ok_or(format!("bad heights {h}"))?),
            (None, None, None) => None,
            (None, lo, hi) => {
                let bound = |v: Option<&str>, d| match v {
                    Some(v) => v.parse().map_err(|_| format!("bad height {v}")),
                    None => Ok(d),
                };
                Some(RangeList(vec![(
                    bound(lo, i32::MIN)?,
                    bound(hi, i32::MAX)?,
                )]))
            }
        };

        let connect = match get("connect") {
            Some("block") => Connect::Block,
            Some("tile") => Connect::Tile,
            Some("state") => Connect::State,
            Some(other) => return Err(format!("unknown connect {other}")),
            None if match_blocks.is_empty() => Connect::Tile,
            None => Connect::Block,
        };

        let orient = match get("orient").map(str::to_ascii_lowercase).as_deref() {
            Some("none") => Orient::None,
            Some("state_axis") => Orient::StateAxis,
            Some("texture") => Orient::Texture,
            // OptiFine's own behaviour.
            _ => match method {
                Method::Horizontal
                | Method::Vertical
                | Method::HorizontalVertical
                | Method::VerticalHorizontal => Orient::StateAxis,
                _ => Orient::None,
            },
        };

        Ok(Some(Self {
            path: path.to_owned(),
            method,
            prioritize: flag("prioritize").unwrap_or(!match_tiles.is_empty()),
            match_tiles,
            match_blocks,
            weight: int("weight", 0),
            faces: get("faces").map_or(faces::ALL, face_bits),
            biomes: get("biomes").map(IdList::parse),
            heights,
            name: match get("name") {
                Some(n) => Some(Name::parse(n).ok_or(format!("bad name {n}"))?),
                None => None,
            },
            connect,
            connect_tiles: get("connectTiles").map_or_else(Vec::new, sprite_list),
            connect_blocks: get("connectBlocks").map_or_else(Vec::new, BlockMatch::parse_list),
            inner_seams: flag("innerSeams") == Some(true),
            weights,
            random_loops: int("randomLoops", 0).clamp(0, 9) as u32,
            symmetry: match get("symmetry") {
                Some("opposite") => Symmetry::Opposite,
                Some("all") => Symmetry::All,
                _ => Symmetry::None,
            },
            linked: flag("linked") == Some(true),
            width,
            height,
            compact_overrides,
            tint_index: int("tintIndex", -1),
            tint_block: get("tintBlock").map(crate::matchers::namespaced),
            layer: match get("layer") {
                Some("translucent") => Layer::Translucent,
                _ => Layer::Cutout,
            },
            orient,
            tiles,
        }))
    }

    /// The rule names this block state (`state` answers its properties).
    pub fn matches_block<'a>(&self, id: &str, state: impl Fn(&str) -> Option<&'a str>) -> bool {
        self.match_blocks.iter().any(|m| m.matches(id, &state))
    }

    /// The rule names this sprite (a texture resource path, [`sprite_path`]).
    pub fn matches_tile(&self, sprite: &str) -> bool {
        self.match_tiles.iter().any(|t| t == sprite)
    }

    /// The tests that depend on where the face is, not on what it is. `face` is a [`faces`] bit;
    /// `name` is the block entity's custom name, if it has one.
    pub fn applies_at(&self, face: u8, y: i32, biome: &str, name: Option<&str>) -> bool {
        self.faces & face != 0
            && self.heights.as_ref().is_none_or(|h| h.contains(y))
            && self.biomes.as_ref().is_none_or(|b| b.matches(biome))
            && self
                .name
                .as_ref()
                .is_none_or(|n| name.is_some_and(|name| n.matches(name)))
    }
}

/// `matchTiles`/`connectTiles` entries as texture resource paths.
fn sprite_list(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .take(MAX_TILES)
        .filter_map(sprite_path)
        .collect()
}

/// A tile name as the texture's resource path: `dirt` is `textures/block/dirt.png`, and a path
/// under `textures/` or `optifine/` is that file. Any other path is in `minecraft` (OptiFine; its
/// `block/` and `item/` sprites in `textures/`) or, after `ns:`, in `ns:textures/` (Continuity).
pub fn sprite_path(name: &str) -> Option<String> {
    let (ns, path, namespaced) = match name.split_once(':') {
        // `Identifier`: an empty namespace is `minecraft`.
        Some(("", path)) => ("minecraft", path, true),
        Some((ns, path)) => (ns, path, true),
        None => ("minecraft", name, false),
    };
    let path = path.strip_suffix(".png").unwrap_or(path);
    let path = match path {
        p if !p.contains('/') => format!("textures/block/{p}"),
        p if p.starts_with("textures/") || p.starts_with("optifine/") => p.to_owned(),
        p if namespaced || p.starts_with("block/") || p.starts_with("item/") => {
            format!("textures/{p}")
        }
        p => p.to_owned(),
    };
    crate::paths::in_root(format!("assets/{}/{path}.png", ns.to_ascii_lowercase()))
}

/// More tiles than any rule needs (a 64 x 64 `repeat`): a range past it is a broken file, not
/// millions of textures.
const MAX_TILES: usize = 4096;

/// `tiles`: `0-3 5 name name.png <skip> <default> path/x.png ~/x /x assets/minecraft/x ns:path`
/// (`/` is `~/`). Numbers and bare
/// names are beside the properties file; ranges count up (or down). A tile outside the resource
/// root fails the list (weights pair with tiles by position), as does a list past [`MAX_TILES`].
fn tile_list(props: &str, value: &str) -> Result<Vec<Tile>, String> {
    let png = |p: &str| {
        if p.ends_with(".png") {
            p.to_owned()
        } else {
            format!("{p}.png")
        }
    };
    let bad = |t: &str| format!("tile {t} is outside the resource root");
    let too_many = format!("more than {MAX_TILES} tiles");
    let mut out = Vec::new();
    for token in value.split_whitespace() {
        // Before every token: a file can't grow millions of paths first.
        if out.len() >= MAX_TILES {
            return Err(too_many);
        }
        match token {
            "<skip>" => out.push(Tile::Skip),
            "<default>" => out.push(Tile::Default),
            // Continuity: a namespaced tile is a sprite location, never a range.
            t if t.contains(':') => out.push(Tile::Texture(sprite_path(t).ok_or(bad(t))?)),
            t => {
                if let Some((a, b)) = t.split_once('-')
                    && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
                {
                    if out.len() + a.abs_diff(b) as usize >= MAX_TILES {
                        return Err(too_many);
                    }
                    let dir = props.rsplit_once('/').map_or("", |(d, _)| d);
                    let tile = |n| Tile::Texture(format!("{dir}/{n}.png"));
                    if a <= b {
                        out.extend((a..=b).map(tile));
                    } else {
                        out.extend((b..=a).rev().map(tile));
                    }
                    continue;
                }
                if let Some(rest) = t.strip_prefix("assets/minecraft/") {
                    // A full path: `minecraft` whatever the file's namespace.
                    let path = crate::paths::in_root(format!("assets/minecraft/{}", png(rest)));
                    out.push(Tile::Texture(path.ok_or(bad(t))?));
                    continue;
                }
                // A leading `/` is OptiFine's folder, as `~/` is.
                let path = match t.strip_prefix('/') {
                    Some(rest) => format!("~/{rest}"),
                    None => t.to_owned(),
                };
                out.push(Tile::Texture(
                    crate::resolve(props, &png(&path)).ok_or(bad(t))?,
                ));
            }
        }
    }
    Ok(out)
}

fn face_bits(value: &str) -> u8 {
    value
        .split_whitespace()
        .map(|f| match f {
            "bottom" | "down" => faces::DOWN,
            "top" | "up" => faces::UP,
            "north" => faces::NORTH,
            "south" => faces::SOUTH,
            "west" => faces::WEST,
            "east" => faces::EAST,
            "sides" => faces::SIDES,
            "all" => faces::ALL,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

/// `heights`: OptiFine's ranges, or Continuity's `a..b`, `a..`, `..b` (either order).
fn height_list(value: &str) -> Option<RangeList> {
    let mut out = Vec::new();
    for token in value.split([' ', ',']).filter(|t| !t.is_empty()) {
        match token.split_once("..") {
            Some((lo, hi)) => {
                // An empty side is open; anything else must be a number.
                let side = |v: &str| match v {
                    "" => Some(None),
                    v => v.parse::<i32>().ok().map(Some),
                };
                out.push(match (side(lo)?, side(hi)?) {
                    (Some(a), Some(b)) => (a.min(b), a.max(b)),
                    (Some(a), None) => (a, i32::MAX),
                    (None, Some(b)) => (i32::MIN, b),
                    (None, None) => return None,
                });
            }
            None => out.extend(RangeList::parse(token)?.0),
        }
    }
    Some(RangeList(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "assets/minecraft/optifine/ctm/glass";

    fn rule(file: &str, text: &str) -> Result<Option<Rule>, String> {
        Rule::parse(&format!("{DIR}/{file}"), &Properties::parse(text))
    }

    fn tex(path: &str) -> Tile {
        Tile::Texture(path.to_owned())
    }

    #[test]
    fn a_glass_rule() {
        let r = rule(
            "glass.properties",
            "matchBlocks=glass white_stained_glass\nmethod=ctm\ntiles=0-46\nconnect=state\nfaces=sides top\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.method, Method::Ctm);
        assert_eq!(r.tiles.len(), 47);
        assert_eq!(r.tiles[0], tex(&format!("{DIR}/0.png")));
        assert_eq!(r.tiles[46], tex(&format!("{DIR}/46.png")));
        assert_eq!(r.connect, Connect::State);
        assert_eq!(r.faces, faces::SIDES | faces::UP);
        assert!(r.matches_block("minecraft:white_stained_glass", |_| None));
        assert!(!r.matches_block("minecraft:glass_pane", |_| None));
        assert!(!r.prioritize);
        assert!(r.applies_at(faces::NORTH, 64, "minecraft:plains", None));
        assert!(!r.applies_at(faces::DOWN, 64, "minecraft:plains", None));
    }

    #[test]
    fn the_file_name_names_what_it_matches() {
        let tile = rule("dirt.properties", "method=random\ntiles=0-3")
            .unwrap()
            .unwrap();
        assert!(tile.matches_tile("assets/minecraft/textures/block/dirt.png"));
        assert_eq!(tile.connect, Connect::Tile);
        assert!(tile.prioritize);
        let block = rule("block_bookshelf.properties", "method=horizontal\ntiles=0-3")
            .unwrap()
            .unwrap();
        assert!(block.matches_block("minecraft:bookshelf", |_| None));
        assert!(block.match_tiles.is_empty());
        assert_eq!(block.orient, Orient::StateAxis);
        // An explicit list replaces the file name.
        let named = rule("x.properties", "matchBlocks=stone\nmethod=fixed\ntiles=0")
            .unwrap()
            .unwrap();
        assert!(named.match_tiles.is_empty());
    }

    #[test]
    fn tile_lists() {
        let r = rule(
            "a.properties",
            "matchTiles=stone\nmethod=random\ntiles=3-1 name other.png <skip> <default> ~/shared/x /y textures/block/z.png assets/minecraft/optifine/w mymod:leaf mymod:optifine/ctm/q 0-x",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            r.tiles,
            [
                tex(&format!("{DIR}/3.png")),
                tex(&format!("{DIR}/2.png")),
                tex(&format!("{DIR}/1.png")),
                tex(&format!("{DIR}/name.png")),
                tex(&format!("{DIR}/other.png")),
                Tile::Skip,
                Tile::Default,
                tex("assets/minecraft/optifine/shared/x.png"),
                tex("assets/minecraft/optifine/y.png"),
                tex("assets/minecraft/textures/block/z.png"),
                tex("assets/minecraft/optifine/w.png"),
                tex("assets/mymod/textures/block/leaf.png"),
                tex("assets/mymod/optifine/ctm/q.png"),
                tex(&format!("{DIR}/0-x.png")),
            ]
        );
    }

    #[test]
    fn tile_counts_per_method() {
        let ok = |m: &str, tiles: &str| {
            rule(
                "a.properties",
                &format!("matchBlocks=stone\nmethod={m}\n{tiles}"),
            )
            .is_ok()
        };
        assert!(ok("ctm", "tiles=0-46"));
        assert!(!ok("ctm", "tiles=0-45"));
        assert!(ok("ctm_compact", "tiles=0-6"));
        assert!(!ok("ctm_compact", "tiles=0-3"));
        assert!(ok("horizontal+vertical", "tiles=0-6"));
        assert!(ok("repeat", "width=3\nheight=2\ntiles=0-5"));
        assert!(!ok("repeat", "width=3\nheight=2\ntiles=0-4"));
        assert!(!ok("repeat", "tiles=0"));
        assert!(ok("overlay", "tiles=0-15 <skip>"));
        assert!(!ok("overlay_fixed", "tiles=<default>"));
        assert!(!ok("spiral", "tiles=0"));
        assert!(!ok("random", ""));
    }

    #[test]
    fn namespaced_sprites() {
        let r = rule(
            "a.properties",
            "matchTiles=mymod:blocks/marble mymod:optifine/ctm/x blocks/marble\nmethod=fixed\ntiles=0",
        )
        .unwrap()
        .unwrap();
        assert!(r.matches_tile("assets/mymod/textures/blocks/marble.png"));
        assert!(r.matches_tile("assets/mymod/optifine/ctm/x.png"));
        assert!(
            rule(
                "a.properties",
                "matchTiles=:glass\nmethod=fixed\ntiles=:dirt"
            )
            .unwrap()
            .unwrap()
            .matches_tile("assets/minecraft/textures/block/glass.png")
        );
        // Without a namespace, a path is in minecraft as written.
        assert!(r.matches_tile("assets/minecraft/blocks/marble.png"));
    }

    #[test]
    fn method_options() {
        let r = rule(
            "a.properties",
            "matchBlocks=stone\nmethod=ctm_compact\ntiles=0-6\nctm.46=5\nctm.17=6\nctm.50=1\nctm.3=9\ninnerSeams=true\nweights=1 2\nheights=(-64)-0 100..\nbiomes=plains !desert\nname=ipattern:*vault*",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            r.compact_overrides.len(),
            2,
            "case 50 and tile 9 do not exist"
        );
        assert!(r.compact_overrides.contains(&(46, 5)));
        assert!(r.inner_seams);
        assert_eq!(r.weights, [1, 2, 1, 1, 1, 1, 1]);
        for (y, ok) in [
            (-64, true),
            (0, true),
            (50, false),
            (100, true),
            (319, true),
        ] {
            assert_eq!(
                r.applies_at(faces::UP, y, "plains", Some("Vault")),
                ok,
                "{y}"
            );
        }
        assert!(!r.applies_at(faces::UP, 0, "plains", None));
        let legacy = rule(
            "a.properties",
            "matchBlocks=stone\nmethod=fixed\ntiles=0\nminHeight=10",
        )
        .unwrap()
        .unwrap();
        assert!(legacy.heights.as_ref().unwrap().contains(10));
        assert!(!legacy.heights.as_ref().unwrap().contains(9));
    }

    #[test]
    fn order_and_continuity_only_files() {
        let mk = |path: &str, text: &str| {
            Rule::parse(path, &Properties::parse(text))
                .unwrap()
                .unwrap()
        };
        let mut rules = vec![
            mk(
                "assets/minecraft/optifine/ctm/b.properties",
                "matchBlocks=stone\nmethod=fixed\ntiles=0",
            ),
            mk(
                "assets/minecraft/optifine/ctm/c.properties",
                "matchBlocks=stone\nmethod=fixed\ntiles=0\nweight=5",
            ),
            mk(
                "assets/minecraft/optifine/ctm/a.properties",
                "matchBlocks=stone\nmethod=fixed\ntiles=0",
            ),
            mk(
                "assets/minecraft/optifine/ctm/z.properties",
                "matchTiles=stone\nmethod=fixed\ntiles=0",
            ),
        ];
        sort(&mut rules);
        let names: Vec<_> = rules
            .iter()
            .map(|r| r.path.rsplit('/').next().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "z.properties",
                "c.properties",
                "a.properties",
                "b.properties"
            ]
        );

        let full = Rule::parse(
            "assets/mymod/optifine/ctm/a.properties",
            &Properties::parse(
                "matchBlocks=stone\nmethod=fixed\ntiles=assets/minecraft/textures/block/stone.png",
            ),
        );
        assert_eq!(
            full.unwrap().unwrap().tiles,
            [tex("assets/minecraft/textures/block/stone.png")]
        );
        let text = "matchBlocks=stone\nmethod=fixed\ntiles=0\noptifineOnly=true";
        assert!(rule("a.properties", text).unwrap().is_none());
        assert!(
            Rule::parse(
                "assets/mymod/optifine/ctm/a.properties",
                &Properties::parse(text)
            )
            .unwrap()
            .is_some()
        );
    }
    #[test]
    fn broken_values() {
        let base = "matchBlocks=furnace\nmethod=random\n";
        let r = |extra: &str| rule("a.properties", &format!("{base}{extra}"));
        // A pack can't ask for billions of tiles, or a pattern too big to count.
        assert!(r("tiles=0-4294967295").is_err());
        assert!(r("tiles=0-2999 3000-5000").is_err());
        assert!(r("tiles=0-4096").is_err());
        assert!(r(&format!("tiles={}0-0", "<skip> ".repeat(4097))).is_err());
        assert_eq!(
            r(&format!("tiles={}", "a ".repeat(4096)))
                .unwrap()
                .unwrap()
                .tiles
                .len(),
            4096
        );
        assert!(r(&format!("tiles={}", "a ".repeat(4097))).is_err());
        // Namespaced tiles stay in the resource root too, and a `\` (written `\\` in the file)
        // is no separator on any platform.
        for bad in ["mymod:../../x", "..:x", r"~/..\\..\\x", r"a\\b"] {
            assert!(r(&format!("tiles={bad}")).is_err(), "{bad}");
        }
        assert!(r("tiles=0\nheights=abc..64").is_err());
        assert!(r("tiles=0\nminHeight=abc").is_err());
        assert_eq!(r("tiles=0-4095").unwrap().unwrap().tiles.len(), 4096);
        let repeat = "matchBlocks=stone\nmethod=repeat\nwidth=65536\nheight=65536\ntiles=0";
        assert!(rule("a.properties", repeat).is_err());
        // A name test that can't be read matches no block, so the rule is dropped.
        assert!(r("tiles=0\nname=regex:[").is_err());
        // Weights pair with tiles by position.
        let w = r("tiles=0-2\nweights=10 x 4294967295").unwrap().unwrap();
        assert_eq!(w.weights, [10, 1, 1]);
        assert!(r("tiles=a ../b c\nweights=1 2 3").is_err());
        assert_eq!(
            r("tiles=0-2\nweights=-5 2 x").unwrap().unwrap().weights,
            [0, 2, 1]
        );
        assert!(
            r("tiles=0-1\nweights=-1 0")
                .unwrap()
                .unwrap()
                .weights
                .is_empty()
        );
        assert!(
            r("tiles=0-2\nweights=0 0 0")
                .unwrap()
                .unwrap()
                .weights
                .is_empty()
        );
        assert!(r("tiles=0\nprioritize=TRUE").unwrap().unwrap().prioritize);
        assert!(r("tiles=0\nheights=..").is_err());
        assert!(r("tiles=0\nconnect=sideways").is_err());
        assert_eq!(r("tiles=0").unwrap().unwrap().connect, Connect::Block);
    }

    #[test]
    fn height_forms() {
        let h = |v: &str| {
            rule(
                "a.properties",
                &format!("matchBlocks=stone\nmethod=fixed\ntiles=0\n{v}"),
            )
            .unwrap()
            .unwrap()
            .heights
            .unwrap()
        };
        let below = h("heights=..10");
        assert!(below.contains(i32::MIN) && below.contains(10) && !below.contains(11));
        let swapped = h("heights=128..-16");
        assert!(swapped.contains(-16) && swapped.contains(128) && !swapped.contains(129));
        let max = h("maxHeight=5");
        assert!(max.contains(-64) && max.contains(5) && !max.contains(6));
    }

    #[test]
    fn loading_a_pack() {
        let root = std::env::temp_dir().join(format!("horizon-ctm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ctm = root.join("assets/minecraft/optifine/ctm");
        std::fs::create_dir_all(ctm.join("b")).unwrap();
        std::fs::write(
            ctm.join("b/block_stone.properties"),
            "method=fixed\ntiles=0",
        )
        .unwrap();
        std::fs::write(ctm.join("a.properties"), "matchBlocks=stone\nmethod=spiral").unwrap();
        // ISO-8859-1, as Java reads it: "É" is one byte.
        let latin1 = b"matchBlocks=furnace\nmethod=fixed\ntiles=0\nname=\xc9p\xe9e\n";
        std::fs::write(ctm.join("c.properties"), latin1).unwrap();
        std::fs::write(ctm.join("z.properties"), "method=fixed\ntiles=0").unwrap();
        let rules = load(&root);
        let names: Vec<_> = rules
            .iter()
            .map(|r| r.path.rsplit('/').next().unwrap())
            .collect();
        // The broken a.properties is left out; the tile rule z comes first.
        assert_eq!(
            names,
            ["z.properties", "block_stone.properties", "c.properties"]
        );
        assert!(rules[2].applies_at(faces::UP, 0, "plains", Some("Épée")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
