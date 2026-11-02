//! OptiFine connected textures (`optifine/ctm`) at mesh time. The active packs' rules are read
//! once at load, their tiles become sprite layers, and each block state and sprite layer gets
//! the short list of rules that can touch it; meshing a face then walks only those lists
//! ([`ConnectedTextures::pick`]). Without CTM files there are no rules and nothing changes.
//!
//! The rules and the tile geometry are `horizon_optifine::ctm`'s.

use crate::sprites::Sprites;
use horizon_optifine::ctm::{
    self, Connect, Method, Orient, Rule, Tile,
    connect::{self as geometry, Axes, Axis, Face},
};
use horizon_world::block_state_properties::BlockProperties;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{collections::HashMap, path::Path};

/// A tile as the mesher draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileLayer {
    Layer {
        layer: u16,
        /// The sprite has fractional alpha (drawn in the translucent pass).
        translucent: bool,
        /// Every texel is opaque (drawn without an alpha test).
        opaque: bool,
    },
    /// Try the next rule.
    Skip,
    /// The face's own sprite.
    Default,
}

/// What a face draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Its own sprite.
    Keep,
    Tile(TileLayer),
    /// `ctm_compact`: a tile per quarter of the sprite (top-left, top-right, bottom-left,
    /// bottom-right).
    Quarters([TileLayer; 4]),
}

/// A tile drawn over a face, after its own (`overlay*` methods).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub layer: u16,
    /// `layer=translucent`: drawn in the translucent pass, else with an alpha cutout.
    pub translucent: bool,
    /// `tintIndex` and the state whose tint it is (`tintBlock`, else the face's block).
    pub tint: Option<(i32, Option<u32>)>,
}

struct LoadedRule {
    rule: Rule,
    tiles: Vec<TileLayer>,
    overlay: OverlayJoin,
}

/// What an overlay rule joins: `connectBlocks` as state ids and `connectTiles` as sprite
/// layers (sorted), and its tint.
#[derive(Default)]
struct OverlayJoin {
    states: Vec<u32>,
    tiles: Vec<u16>,
    tint: Option<(i32, Option<u32>)>,
}

impl OverlayJoin {
    fn any(&self) -> bool {
        !self.states.is_empty() || !self.tiles.is_empty()
    }
}

#[derive(Clone, Copy, Default)]
struct StateInfo {
    /// Index of the state's block (`connect=block`).
    block: u32,
    axis: Option<Axis>,
    /// `half=upper`: `linked` random tiles follow the lower half.
    upper: bool,
    /// A pane's arms (`north`, `east`, `south`, `west` true), as bits of [`ARMS`].
    arms: u8,
    /// Rules name the state or one of its sides' sprites ([`ConnectedTextures::stacked_pane_hides`]).
    has_rules: bool,
}

/// A pane's arm properties, in [`StateInfo::arms`]' bit order.
const ARMS: [&str; 4] = ["north", "east", "south", "west"];

#[derive(Default)]
pub struct ConnectedTextures {
    rules: Vec<LoadedRule>,
    /// By block state, then by sprite layer: ranges of `lists` (rule indices, in rule order).
    by_state: Vec<(u32, u32)>,
    by_layer: Vec<(u32, u32)>,
    lists: Vec<u16>,
    states: Vec<StateInfo>,
    /// By block state: the sprite of its model's face on each side (`connect=tile`); empty
    /// when no rule connects by tile.
    face_layers: Vec<[u16; 6]>,
}

/// `list` appended to `lists`, as its range there.
fn push_list(lists: &mut Vec<u16>, list: &[u16]) -> (u32, u32) {
    let start = lists.len() as u32;
    lists.extend_from_slice(list);
    (start, list.len() as u32)
}

/// The sprite name of a texture resource path: `ns:path` under `textures/`, else the path
/// itself (as [`Sprites::insert_file`] names a tile).
fn sprite_name(resource: &str) -> String {
    let parts = resource
        .strip_prefix("assets/")
        .and_then(|p| p.split_once('/'))
        .and_then(|(ns, p)| Some((ns, p.strip_prefix("textures/")?.strip_suffix(".png")?)));
    match parts {
        Some((ns, path)) => format!("{ns}:{path}"),
        None => resource.to_owned(),
    }
}

impl ConnectedTextures {
    /// The rules under `root`, with their tiles inserted into `sprites` (before it finishes).
    /// `face_layer` answers a block state's sprite on a side, for rules that connect by tile.
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
        mut face_layer: impl FnMut(BlockStateId, Face) -> Option<u32>,
    ) -> Self {
        let mut rules = ctm::load(root);
        // Rules with a custom name load no tiles: section meshes see no block entity.
        rules.retain(|r| r.name.is_none());
        // Rule indices are `u16`.
        rules.truncate(usize::from(u16::MAX));
        if rules.is_empty() {
            return Self::default();
        }
        // The tiles decode side by side; the loop below inserts them in rule order, under the
        // same names.
        let tiles: Vec<String> = rules
            .iter()
            .flat_map(|rule| &rule.tiles)
            .filter_map(|tile| match tile {
                Tile::Texture(path) => Some(sprite_name(path)),
                _ => None,
            })
            .collect();
        sprites.prefetch(root, tiles.iter().map(String::as_str));
        let loaded = rules
            .into_iter()
            .filter_map(|rule| {
                let tiles = rule
                    .tiles
                    .iter()
                    .map(|tile| match tile {
                        Tile::Skip => Ok(TileLayer::Skip),
                        Tile::Default => Ok(TileLayer::Default),
                        Tile::Texture(path) => {
                            // A tile among the textures is that sprite, under its own name.
                            let layer = match sprite_name(path) {
                                name if name != *path => sprites.insert(root, &name)?,
                                _ => sprites.insert_file(root, path)?,
                            };
                            Ok(TileLayer::Layer {
                                layer: u16::try_from(layer)?,
                                translucent: sprites.translucent[layer as usize],
                                opaque: sprites.opaque[layer as usize],
                            })
                        }
                    })
                    .collect::<crate::Result<Vec<_>>>();
                match tiles {
                    Ok(tiles) => Some(LoadedRule {
                        rule,
                        tiles,
                        overlay: OverlayJoin::default(),
                    }),
                    Err(e) => {
                        log::warn!("Horizon: connected textures {}: a tile: {e}", rule.path);
                        None
                    }
                }
            })
            .collect();
        let states = (0..registry.len() as u32).map(|id| {
            registry
                .state(BlockStateId(id))
                .map(|s| (s.name.as_str(), &s.properties))
        });
        let mut out = Self::build(loaded, states, |name| sprites.layers.get(name).copied());
        if out
            .rules
            .iter()
            .any(|r| r.rule.connect == Connect::Tile || !r.overlay.tiles.is_empty())
        {
            out.face_layers = (0..registry.len() as u32)
                .map(|id| {
                    Face::ALL
                        .map(|f| face_layer(BlockStateId(id), f).map_or(u16::MAX, |l| l as u16))
                })
                .collect();
        }
        // Panes whose sprites tile rules name (`matchTiles=glass`) are connected too, by a rule
        // that can draw on them: here, one naming no block (a rule naming the pane is in its
        // own list already).
        for id in (0..out.states.len()).filter(|_| !out.by_layer.is_empty()) {
            let pane = registry
                .state(BlockStateId(id as u32))
                .is_some_and(|s| s.name.ends_with("glass_pane"));
            if pane && !out.states[id].has_rules {
                out.states[id].has_rules = Face::ALL.into_iter().any(|f| {
                    face_layer(BlockStateId(id as u32), f).is_some_and(|l| {
                        out.list(&out.by_layer, l as usize)
                            .iter()
                            .any(|&i| out.rules[usize::from(i)].rule.match_blocks.is_empty())
                    })
                });
            }
        }
        out
    }

    /// The lookups for `rules` (already ordered) over every block state (`None` for an id the
    /// registry skips), `layer` answering a sprite name's layer.
    fn build<'a>(
        rules: Vec<LoadedRule>,
        states: impl Iterator<Item = Option<(&'a str, &'a BlockProperties)>>,
        layer: impl Fn(&str) -> Option<u32>,
    ) -> Self {
        let mut out = Self {
            rules,
            ..Self::default()
        };
        let states: Vec<_> = states.collect();
        // Each block's states, for `connectBlocks` and `tintBlock`.
        let mut block_states: HashMap<&str, Vec<u32>> = HashMap::new();
        if out.rules.iter().any(|r| r.rule.method.is_overlay()) {
            for (id, state) in states.iter().enumerate() {
                if let Some((name, _)) = state {
                    block_states.entry(name).or_default().push(id as u32);
                }
            }
        }
        for r in out.rules.iter_mut().filter(|r| r.rule.method.is_overlay()) {
            let rule = &r.rule;
            // `connectBlocks` and `connectTiles` are "only for method overlay".
            if rule.method == Method::Overlay {
                for m in &rule.connect_blocks {
                    let candidates = block_states
                        .get(m.id.as_str())
                        .map_or(&[][..], Vec::as_slice);
                    r.overlay.states.extend(candidates.iter().filter(|&&id| {
                        states[id as usize].is_some_and(|(name, props)| {
                            m.matches(name, |k| props.get(k).map(String::as_str))
                        })
                    }));
                }
                r.overlay.states.sort_unstable();
                r.overlay.states.dedup();
                r.overlay.tiles = rule
                    .connect_tiles
                    .iter()
                    .filter_map(|t| layer(&sprite_name(t)).map(|l| l as u16))
                    .collect();
                r.overlay.tiles.sort_unstable();
            }
            // A `tintBlock` the game doesn't have tints nothing.
            let tint = match rule.tint_block.as_deref() {
                None => Some(None),
                Some(block) => block_states.get(block).map(|ids| Some(ids[0])),
            };
            r.overlay.tint = tint
                .filter(|_| rule.tint_index >= 0)
                .map(|b| (rule.tint_index, b));
        }
        let mut blocks = HashMap::new();
        let mut by_layer: Vec<Vec<u16>> = Vec::new();
        // Block rules by the block each entry names, so a state tests only its own.
        let mut by_block: HashMap<&str, Vec<u16>> = HashMap::new();
        for (i, r) in out.rules.iter().enumerate() {
            let i = i as u16;
            for tile in &r.rule.match_tiles {
                if let Some(l) = layer(&sprite_name(tile)) {
                    let l = l as usize;
                    if by_layer.len() <= l {
                        by_layer.resize(l + 1, Vec::new());
                    }
                    if by_layer[l].last() != Some(&i) {
                        by_layer[l].push(i);
                    }
                }
            }
            for m in &r.rule.match_blocks {
                let list = by_block.entry(m.id.as_str()).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
        }
        for list in by_layer {
            let range = push_list(&mut out.lists, &list);
            out.by_layer.push(range);
        }
        let mut list = Vec::new();
        for state in states {
            let Some((name, props)) = state else {
                out.by_state.push((0, 0));
                out.states.push(StateInfo::default());
                continue;
            };
            let next = blocks.len() as u32;
            let block = *blocks.entry(name).or_insert(next);
            let prop = |k: &str| props.get(k).map(String::as_str);
            out.states.push(StateInfo {
                block,
                axis: match prop("axis") {
                    Some("x") => Some(Axis::X),
                    Some("y") => Some(Axis::Y),
                    Some("z") => Some(Axis::Z),
                    _ => None,
                },
                upper: prop("half") == Some("upper"),
                arms: ARMS
                    .iter()
                    .enumerate()
                    .filter(|&(_, arm)| prop(arm) == Some("true"))
                    .fold(0, |bits, (i, _)| bits | 1 << i),
                // Set below, with the state's rule list.
                has_rules: false,
            });
            list.clear();
            if let Some(candidates) = by_block.get(name) {
                list.extend(
                    candidates
                        .iter()
                        .filter(|&&i| out.rules[usize::from(i)].rule.matches_block(name, prop)),
                );
            }
            out.states.last_mut().expect("pushed above").has_rules = !list.is_empty();
            let range = push_list(&mut out.lists, &list);
            out.by_state.push(range);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether a pane's top or bottom strip, centred at `centre` (block-local x, z), is hidden by
    /// the pane `other` stacked on it (Continuity's "Glass Pane Culling Fix"): only for a pane
    /// with connected-texture rules, under or over the same block, where `other` has the same
    /// part (the post, or the arm toward the strip).
    pub fn stacked_pane_hides(&self, state: u32, other: u32, [x, z]: [f32; 2]) -> bool {
        let (Some(own), Some(theirs)) = (
            self.states.get(state as usize),
            self.states.get(other as usize),
        ) else {
            return false;
        };
        if own.block != theirs.block || !own.has_rules {
            return false;
        }
        let (dx, dz) = (x - 0.5, z - 0.5);
        // The post is 2 pixels wide; an arm runs from it to the block's edge.
        if dx.abs() <= 1.0 / 16.0 && dz.abs() <= 1.0 / 16.0 {
            return true;
        }
        let arm = if dz.abs() > dx.abs() {
            if dz < 0.0 { 0 } else { 2 }
        } else if dx > 0.0 {
            1
        } else {
            3
        };
        theirs.arms & 1 << arm != 0
    }

    fn list(&self, ranges: &[(u32, u32)], i: usize) -> &[u16] {
        ranges.get(i).map_or(&[], |&(start, len)| {
            &self.lists[start as usize..(start + len) as usize]
        })
    }

    /// What the `face` of the block `state` at `pos` draws, for a quad with sprite `layer`.
    /// `neighbour` answers the state at an offset from the block; `biome` its biome, asked only
    /// by rules that test one; `texture` the quad's own texture directions, asked only by rules
    /// with `orient=texture`.
    pub fn pick<'b>(
        &self,
        state: u32,
        layer: u16,
        face: Face,
        pos: [i32; 3],
        neighbour: impl Fn([i32; 3]) -> u32,
        biome: impl Fn() -> Option<&'b str>,
        texture: impl Fn() -> Option<Axes>,
        mut overlay: impl FnMut(Overlay),
    ) -> Pick {
        let (mut by_state, mut by_layer) = (
            self.list(&self.by_state, state as usize),
            self.list(&self.by_layer, layer as usize),
        );
        // Both lists are in rule order: walk them as one. A rule naming blocks and tiles needs
        // both (it is in both lists); one naming either needs that one.
        loop {
            let (i, block, tile) = match (by_state.split_first(), by_layer.split_first()) {
                (None, None) => return Pick::Keep,
                (Some((&x, rest)), Some((&y, other))) if x == y => {
                    (by_state, by_layer) = (rest, other);
                    (x, true, true)
                }
                (Some((&x, rest)), Some((&y, _))) if x < y => {
                    by_state = rest;
                    (x, true, false)
                }
                (Some((&x, rest)), None) => {
                    by_state = rest;
                    (x, true, false)
                }
                (_, Some((&y, rest))) => {
                    by_layer = rest;
                    (y, false, true)
                }
            };
            let r = &self.rules[usize::from(i)];
            if (block || r.rule.match_blocks.is_empty())
                && (tile || r.rule.match_tiles.is_empty())
                && let Some(pick) = self.apply(
                    r,
                    state,
                    layer,
                    face,
                    pos,
                    &neighbour,
                    &biome,
                    &texture,
                    &mut overlay,
                )
            {
                // "Tiles output by CTM rules can also be matched by another rule": once, by the
                // tile rules naming the picked tile (and this block, if they name blocks).
                // A tile on the face's own sprite (`tiles=stone`) is no new tile.
                let Pick::Tile(TileLayer::Layer { layer: picked, .. }) = pick else {
                    return pick;
                };
                if picked == layer {
                    return pick;
                }
                let own = self.list(&self.by_state, state as usize);
                // Rules tried on the face's sprite already (those before this one) are not tried
                // again: an overlay would draw twice.
                let first = self.list(&self.by_layer, layer as usize);
                let again = self
                    .list(&self.by_layer, picked as usize)
                    .iter()
                    .map(|&j| (j, &self.rules[usize::from(j)]))
                    .filter(|&(j, r)| {
                        j != i
                            && !(j < i && first.contains(&j))
                            && (r.rule.match_blocks.is_empty() || own.contains(&j))
                    });
                for (_, r) in again {
                    // Joins by tile still compare the face's own sprite with its neighbours'.
                    match self.apply(
                        r,
                        state,
                        layer,
                        face,
                        pos,
                        &neighbour,
                        &biome,
                        &texture,
                        &mut overlay,
                    ) {
                        Some(Pick::Keep) => return pick,
                        Some(chained) => return chained,
                        None => {}
                    }
                }
                return pick;
            }
        }
    }

    /// One rule on a face: `None` when it doesn't apply, picks `<skip>` or draws an overlay
    /// (handed to `overlay`: the rules after it still choose the face's own sprite).
    #[allow(clippy::too_many_arguments)]
    fn apply<'b>(
        &self,
        r: &LoadedRule,
        state: u32,
        layer: u16,
        face: Face,
        pos: [i32; 3],
        neighbour: &impl Fn([i32; 3]) -> u32,
        biome: &impl Fn() -> Option<&'b str>,
        texture: &impl Fn() -> Option<Axes>,
        overlay: &mut impl FnMut(Overlay),
    ) -> Option<Pick> {
        let rule = &r.rule;
        // `Rule::applies_at`, with the biome looked up only for rules that test one. Rules with
        // a custom name are not loaded.
        if rule.faces & face.bit() == 0
            || rule.heights.as_ref().is_some_and(|h| !h.contains(pos[1]))
        {
            return None;
        }
        if let Some(biomes) = &rule.biomes
            && !biome().is_some_and(|b| biomes.matches(b))
        {
            return None;
        }
        let info = self.states.get(state as usize).copied().unwrap_or_default();
        let join = &r.overlay;
        let joins = |offset: [i32; 3]| {
            let other = neighbour(offset);
            if join.any() {
                // An overlay joins the blocks it is drawn beside.
                return join.states.binary_search(&other).is_ok()
                    || self
                        .face_layers
                        .get(other as usize)
                        .is_some_and(|f| join.tiles.binary_search(&f[face as usize]).is_ok());
            }
            match rule.connect {
                Connect::Block => {
                    self.states.get(other as usize).map(|s| s.block) == Some(info.block)
                }
                Connect::State => other == state,
                Connect::Tile => self
                    .face_layers
                    .get(other as usize)
                    .is_some_and(|f| f[face as usize] == layer),
            }
        };
        let axes = |face| match (rule.orient, info.axis) {
            (Orient::StateAxis, Some(axis)) => Axes::column(face, axis),
            // The quad's own texture directions, whatever face `symmetry` stands it for.
            // Not for `overlay`, which Continuity's `orient` leaves out: its 17 tiles always face
            // the standard way.
            (Orient::Texture, _) if rule.method != Method::Overlay => {
                texture().unwrap_or(Axes::cube(face))
            }
            _ => Axes::cube(face),
        };
        let mask = || geometry::neighbours(axes(face), face.normal(), rule.inner_seams, joins);
        let tile = |i: usize| r.tiles.get(i).copied().unwrap_or(TileLayer::Default);
        let mut draw = |t: TileLayer| {
            if let TileLayer::Layer { layer, .. } = t {
                overlay(Overlay {
                    layer,
                    translucent: rule.layer == ctm::Layer::Translucent,
                    tint: join.tint,
                });
            }
        };
        // An overlay picks its tile as the method it is the overlay of.
        let method = match rule.method {
            Method::Overlay => {
                // The 17 transition tiles, toward the blocks it names (none: it draws nothing).
                if join.any() {
                    let mask = geometry::neighbours(axes(face), face.normal(), false, joins);
                    geometry::overlay_tiles(mask).for_each(|i| draw(tile(i)));
                }
                return None;
            }
            Method::OverlayCtm => Method::Ctm,
            Method::OverlayRandom => Method::Random,
            Method::OverlayRepeat => Method::Repeat,
            Method::OverlayFixed => Method::Fixed,
            Method::OverlayHorizontal => Method::Horizontal,
            Method::OverlayVertical => Method::Vertical,
            Method::OverlayHorizontalVertical => Method::HorizontalVertical,
            Method::OverlayVerticalHorizontal => Method::VerticalHorizontal,
            m => m,
        };
        let index = match method {
            Method::Ctm => geometry::ctm_index(mask()),
            Method::CtmCompact => {
                let mask = mask();
                let case = geometry::ctm_index(mask) as u8;
                match rule.compact_overrides.iter().find(|&&(c, _)| c == case) {
                    Some(&(_, t)) => t,
                    None => {
                        let quarters = geometry::compact_quarters(mask).map(|q| tile(q.into()));
                        return if quarters.contains(&TileLayer::Skip) {
                            None
                        } else {
                            Some(Pick::Quarters(quarters))
                        };
                    }
                }
            }
            Method::Horizontal => geometry::horizontal(mask()),
            Method::Vertical => geometry::vertical(mask()),
            Method::HorizontalVertical => geometry::horizontal_vertical(mask()),
            Method::VerticalHorizontal => geometry::vertical_horizontal(mask()),
            Method::Top => {
                // Only sides join the block above: the top and bottom go on to the next rule.
                if matches!(face, Face::Up | Face::Down) {
                    return None;
                }
                if !joins([0, 1, 0]) {
                    return Some(Pick::Keep);
                }
                0
            }
            Method::Fixed => 0,
            Method::Random => {
                let face = geometry::symmetric_face(face, rule.symmetry);
                let pos = if rule.linked && info.upper {
                    [pos[0], pos[1] - 1, pos[2]]
                } else {
                    pos
                };
                geometry::random_index(pos, face, rule.random_loops, &rule.weights, r.tiles.len())
            }
            Method::Repeat => {
                let face = geometry::symmetric_face(face, rule.symmetry);
                geometry::repeat_index(pos, axes(face), rule.width, rule.height)
            }
            // Every overlay was mapped to its method above.
            _ => return None,
        };
        if rule.method.is_overlay() {
            draw(tile(index));
            return None;
        }
        match tile(index) {
            TileLayer::Skip => None,
            TileLayer::Default => Some(Pick::Keep),
            t => Some(Pick::Tile(t)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_optifine::Properties;

    /// A tile whose layer says which tile of its rule it is: rule `r`'s tile `i` is layer
    /// `1000 * (r + 1) + i`.
    fn rules(files: &[(&str, &str)]) -> Vec<LoadedRule> {
        let mut rules: Vec<Rule> = files
            .iter()
            .map(|(path, text)| {
                Rule::parse(
                    &format!("assets/minecraft/optifine/ctm/{path}"),
                    &Properties::parse(text),
                )
                .unwrap()
                .unwrap()
            })
            .collect();
        ctm::sort(&mut rules);
        rules
            .into_iter()
            .enumerate()
            .map(|(n, rule)| LoadedRule {
                overlay: OverlayJoin::default(),
                tiles: rule
                    .tiles
                    .iter()
                    .enumerate()
                    .map(|(i, t)| match t {
                        Tile::Skip => TileLayer::Skip,
                        Tile::Default => TileLayer::Default,
                        Tile::Texture(_) => TileLayer::Layer {
                            layer: (1000 * (n + 1) + i) as u16,
                            translucent: false,
                            opaque: false,
                        },
                    })
                    .collect(),
                rule,
            })
            .collect()
    }

    /// State 0 air, 1 glass, 2 stone, 3 and 4 oak logs (axis y, x), 5 and 6 a door (lower,
    /// upper); the stone sprite is layer 7, glass 3.
    fn world(files: &[(&str, &str)]) -> ConnectedTextures {
        let props = |pairs: &[(&str, &str)]| -> BlockProperties {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let states = [
            ("minecraft:air", props(&[])),
            ("minecraft:glass", props(&[])),
            ("minecraft:stone", props(&[])),
            ("minecraft:oak_log", props(&[("axis", "y")])),
            ("minecraft:oak_log", props(&[("axis", "x")])),
            ("minecraft:oak_door", props(&[("half", "lower")])),
            ("minecraft:oak_door", props(&[("half", "upper")])),
        ];
        ConnectedTextures::build(
            rules(files),
            states.iter().map(|(n, p)| Some((*n, p))),
            |name| match name {
                "minecraft:block/stone" => Some(7),
                "minecraft:block/glass" => Some(3),
                // The tile `b0` of a rule second in order (`a_picked_tile_can_be_picked_again`).
                "assets/minecraft/optifine/ctm/b0.png" => Some(2000),
                // And `a0` of the first rule.
                "assets/minecraft/optifine/ctm/a0.png" => Some(1000),
                _ => None,
            },
        )
    }

    fn tile(layer: u16) -> Pick {
        Pick::Tile(TileLayer::Layer {
            layer,
            translucent: false,
            opaque: false,
        })
    }

    /// `pick` on `face` of the block at the origin among `blocks` (offset, state).
    fn pick_in(
        ctm: &ConnectedTextures,
        state: u32,
        layer: u16,
        face: Face,
        blocks: &[([i32; 3], u32)],
    ) -> Pick {
        let at = |o: [i32; 3]| blocks.iter().find(|(p, _)| *p == o).map_or(0, |b| b.1);
        ctm.pick(state, layer, face, [0, 64, 0], at, || None, || None, |_| {})
    }

    const GLASS: &str = "matchBlocks=glass\nmethod=ctm\ntiles=0-46";

    #[test]
    fn glass_joins_its_neighbours() {
        let ctm = world(&[("glass.properties", GLASS)]);
        // Alone: tile 0. A north face's right is west: glass there is the left end of a row.
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &[]), tile(1000));
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(1001)
        );
        // Glass all around in the face's plane: the middle tile, 26.
        let ring: Vec<_> = [-1, 0, 1]
            .into_iter()
            .flat_map(|x| [-1, 0, 1].map(|y| ([x, y, 0], 1)))
            .collect();
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &ring), tile(1026));
        // Stone beside it doesn't join; stone has no rule.
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 2)]),
            tile(1000)
        );
        assert_eq!(pick_in(&ctm, 2, 7, Face::North, &[]), Pick::Keep);
    }

    #[test]
    fn rule_order_faces_and_skips() {
        let ctm = world(&[
            (
                "a_sides.properties",
                "matchBlocks=stone\nmethod=fixed\ntiles=<skip>\nfaces=sides",
            ),
            ("b_glass.properties", GLASS),
            ("stone.properties", "method=fixed\ntiles=x\nfaces=top"),
            ("z.properties", "matchBlocks=stone\nmethod=fixed\ntiles=y"),
        ]);
        // The tile rule (stone.properties, by its file name) comes first, on the top face only.
        assert_eq!(pick_in(&ctm, 2, 7, Face::Up, &[]), tile(1000));
        // Sides: a_sides skips, so z draws.
        assert_eq!(pick_in(&ctm, 2, 7, Face::North, &[]), tile(4000));
        // The tile rule follows the sprite, whatever the block: glass with the stone sprite.
        assert_eq!(pick_in(&ctm, 1, 7, Face::Up, &[]), tile(1000));
    }

    #[test]
    fn compact_quarters_and_overrides() {
        let ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=ctm_compact\ntiles=0-5\nctm.1=5",
        )]);
        let layer = |i: u16| TileLayer::Layer {
            layer: 1000 + i,
            translucent: false,
            opaque: false,
        };
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[]),
            Pick::Quarters([layer(0); 4])
        );
        // Joined right only is case 1, replaced whole by tile 5.
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(1005)
        );
        // Joined left only (east of a north face): quarters [3, 0, 3, 0].
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([1, 0, 0], 1)]),
            Pick::Quarters([layer(3), layer(0), layer(3), layer(0)])
        );
    }

    #[test]
    fn logs_turn_with_their_axis() {
        let ctm = world(&[(
            "log.properties",
            "matchBlocks=oak_log\nmethod=vertical\ntiles=0-3",
        )]);
        // An upright log with a log above: the bottom of a column.
        assert_eq!(
            pick_in(&ctm, 3, 3, Face::North, &[([0, 1, 0], 3)]),
            tile(1000)
        );
        // A log lying along x: on its north face the texture's up points east, so a log to the
        // east (same block, other axis state) is above it.
        assert_eq!(
            pick_in(&ctm, 4, 3, Face::North, &[([1, 0, 0], 4)]),
            tile(1000)
        );
        assert_eq!(
            pick_in(&ctm, 4, 3, Face::North, &[([0, 1, 0], 4)]),
            tile(1003)
        );
    }

    #[test]
    fn top_and_connect_state() {
        let ctm = world(&[(
            "block_oak_log.properties",
            "method=top\ntiles=0\nconnect=state",
        )]);
        assert_eq!(
            pick_in(&ctm, 3, 3, Face::North, &[([0, 1, 0], 3)]),
            tile(1000)
        );
        // The other axis is another state.
        assert_eq!(
            pick_in(&ctm, 3, 3, Face::North, &[([0, 1, 0], 4)]),
            Pick::Keep
        );
        assert_eq!(pick_in(&ctm, 3, 3, Face::Up, &[([0, 1, 0], 3)]), Pick::Keep);
    }

    #[test]
    fn linked_random_follows_the_lower_half() {
        let ctm = world(&[(
            "door.properties",
            "matchBlocks=oak_door\nmethod=random\ntiles=0-7\nlinked=true",
        )]);
        let mut tiles = std::collections::HashSet::new();
        for x in 0..20 {
            let lower = ctm.pick(
                5,
                3,
                Face::North,
                [x, 64, 0],
                |_| 0,
                || None,
                || None,
                |_| {},
            );
            let upper = ctm.pick(
                6,
                3,
                Face::North,
                [x, 65, 0],
                |_| 0,
                || None,
                || None,
                |_| {},
            );
            assert_eq!(lower, upper, "{x}");
            tiles.insert(format!("{lower:?}"));
        }
        assert!(tiles.len() > 3, "the doors are not all alike");
    }

    #[test]
    fn heights_and_biomes() {
        let ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=fixed\ntiles=0\nheights=0-63\nbiomes=desert",
        )]);
        let at = |y, biome: &'static str| {
            ctm.pick(
                1,
                3,
                Face::Up,
                [0, y, 0],
                |_| 0,
                || Some(biome),
                || None,
                |_| {},
            )
        };
        assert_eq!(at(10, "minecraft:desert"), tile(1000));
        assert_eq!(at(64, "minecraft:desert"), Pick::Keep);
        assert_eq!(at(10, "minecraft:plains"), Pick::Keep);
    }

    #[test]
    fn connect_by_tile_reads_the_neighbours_same_side() {
        let mut ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=horizontal\nconnect=tile\ntiles=0-3",
        )]);
        // Stone shows the glass sprite (3) on its north side; the door only on its south.
        let mut faces = vec![[9u16; 6]; 7];
        faces[1][Face::North as usize] = 3;
        faces[2][Face::North as usize] = 3;
        faces[5][Face::South as usize] = 3;
        ctm.face_layers = faces;
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 2)]),
            tile(1000)
        );
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 5)]),
            tile(1003)
        );
    }

    #[test]
    fn repeat_and_rows_then_columns() {
        let ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=repeat\nwidth=2\nheight=2\ntiles=0-3",
        )]);
        let at = |pos| ctm.pick(1, 3, Face::North, pos, |_| 0, || None, || None, |_| {});
        assert_eq!(at([0, 64, 0]), tile(1000));
        // The north face's right is west; rows run down.
        assert_eq!(at([-1, 64, 0]), tile(1001));
        assert_eq!(at([0, 63, 0]), tile(1002));
        let hv = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=horizontal+vertical\ntiles=0-6",
        )]);
        // Alone in its row, glass above: the column's bottom, tile 4.
        assert_eq!(
            pick_in(&hv, 1, 3, Face::North, &[([0, 1, 0], 1)]),
            tile(1004)
        );
        assert_eq!(
            pick_in(&hv, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(1000)
        );
    }

    #[test]
    fn skips_defaults_and_narrowed_rules() {
        let ctm = world(&[
            (
                "a.properties",
                "matchBlocks=glass\nmethod=ctm_compact\ntiles=<skip> 1-4",
            ),
            (
                "b.properties",
                "matchBlocks=glass\nmethod=fixed\ntiles=0\nfaces=sides",
            ),
            (
                "c.properties",
                "matchBlocks=glass\nmethod=fixed\ntiles=<default>",
            ),
            ("d.properties", "matchBlocks=glass\nmethod=fixed\ntiles=0"),
            (
                "e.properties",
                "matchBlocks=stone\nmatchTiles=glass_pane_top\nmethod=fixed\ntiles=0",
            ),
        ]);
        // e names a tile, so it is tried first (tile 1000s), but needs a stone with the pane
        // sprite. a's skipped quarter drops it; b (3000s) draws the sides; on top c draws the
        // glass itself, which stops d.
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &[]), tile(3000));
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), Pick::Keep);
        assert_eq!(pick_in(&ctm, 2, 7, Face::Up, &[]), Pick::Keep);
        let both = world(&[(
            "e.properties",
            "matchBlocks=glass\nmatchTiles=stone\nmethod=fixed\ntiles=0",
        )]);
        assert_eq!(pick_in(&both, 1, 7, Face::Up, &[]), tile(1000));
        assert_eq!(pick_in(&both, 1, 3, Face::Up, &[]), Pick::Keep);
        assert_eq!(pick_in(&both, 2, 7, Face::Up, &[]), Pick::Keep);
        // `top` passes the top face on.
        let top = world(&[
            ("block_glass.properties", "method=top\ntiles=0"),
            (
                "z.properties",
                "matchBlocks=glass\nmethod=fixed\ntiles=0\nfaces=top",
            ),
        ]);
        assert_eq!(pick_in(&top, 1, 3, Face::Up, &[]), tile(2000));
    }

    /// The overlays `pick` hands out, with what it picks for the face itself.
    fn overlays_in(
        ctm: &ConnectedTextures,
        state: u32,
        face: Face,
        blocks: &[([i32; 3], u32)],
    ) -> (Vec<u16>, Pick) {
        let at = |o: [i32; 3]| blocks.iter().find(|(p, _)| *p == o).map_or(0, |b| b.1);
        let mut out = Vec::new();
        let pick = ctm.pick(
            state,
            3,
            face,
            [0, 64, 0],
            at,
            || None,
            || None,
            |o| out.push(o.layer),
        );
        (out, pick)
    }

    #[test]
    fn overlays_draw_toward_the_blocks_they_name() {
        // Stone gets glass's edge where glass is beside it; then a fixed tile for the stone.
        let ctm = world(&[
            (
                "a_overlay.properties",
                "matchBlocks=stone\nmethod=overlay\nconnectBlocks=glass\ntiles=0-16\ntintIndex=0\ntintBlock=oak_log",
            ),
            ("b.properties", "matchBlocks=stone\nmethod=fixed\ntiles=0"),
        ]);
        // Nothing beside: no overlay, and b draws.
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &[]), (vec![], tile(2000)));
        // Glass to the north of a top face (its texture's up): tile 15. Glass east and west
        // too: one tile for the three sides, 13.
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &[([0, 0, -1], 1)]).0, [1015]);
        let three = [([0, 0, -1], 1), ([1, 0, 0], 1), ([-1, 0, 0], 1)];
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &three).0, [1013]);
        // Glass only at the north-east corner: the corner tile, 14.
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &[([1, 0, -1], 1)]).0, [1014]);
        // Stone beside stone draws nothing.
        assert_eq!(
            overlays_in(&ctm, 2, Face::Up, &[([0, 0, -1], 2)]).0,
            [0u16; 0]
        );
        let mut tint = None;
        ctm.pick(
            2,
            3,
            Face::Up,
            [0, 64, 0],
            |o| if o == [0, 0, -1] { 1 } else { 0 },
            || None,
            || None,
            |o| {
                tint = Some((o.tint, o.translucent));
            },
        );
        assert_eq!(
            tint,
            Some((Some((0, Some(3))), false)),
            "oak_log's first state is 3"
        );
    }

    #[test]
    fn overlay_joins_layers_and_tints() {
        let mut ctm = world(&[
            (
                "a.properties",
                "matchBlocks=stone\nmethod=overlay\nconnectTiles=glass\ntiles=0-16\nlayer=translucent\ntintIndex=0\ntintBlock=no_such_block",
            ),
            (
                "b.properties",
                "matchBlocks=glass\nmethod=overlay_ctm\nconnectBlocks=stone\ntiles=0-46",
            ),
        ]);
        // connectTiles joins neighbours showing a named sprite on the same side: the door to the
        // north shows glass (sprite 3) on top.
        let mut faces = vec![[9u16; 6]; 7];
        faces[5][Face::Up as usize] = 3;
        ctm.face_layers = faces;
        let mut got = Vec::new();
        let door_north = |o: [i32; 3]| if o == [0, 0, -1] { 5 } else { 0 };
        ctm.pick(
            2,
            7,
            Face::Up,
            [0, 64, 0],
            door_north,
            || None,
            || None,
            |o| got.push(o),
        );
        assert_eq!(
            got,
            [Overlay {
                layer: 1015,
                translucent: true,
                // A tintBlock the game lacks tints nothing.
                tint: None,
            }]
        );
        // connectBlocks is only for `overlay`: overlay_ctm joins glass by `connect`, not stone.
        let (layers, _) = overlays_in(&ctm, 1, Face::North, &[([-1, 0, 0], 2)]);
        assert_eq!(layers, [2000]);
        let (layers, _) = overlays_in(&ctm, 1, Face::North, &[([-1, 0, 0], 1)]);
        assert_eq!(layers, [2001]);
    }

    #[test]
    fn overlay_methods_pick_like_their_own() {
        let ctm = world(&[
            (
                "a.properties",
                "matchBlocks=glass\nmethod=overlay_ctm\ntiles=0-46\nlayer=translucent",
            ),
            (
                "b.properties",
                "matchBlocks=glass\nmethod=overlay_fixed\ntiles=<skip>",
            ),
            (
                "c.properties",
                "matchBlocks=glass\nmethod=overlay_fixed\ntiles=0",
            ),
        ]);
        // overlay_ctm joins glass (connect=block): the left end of a row, tile 1; b's <skip>
        // draws nothing; c draws; the face keeps its own sprite.
        let (layers, pick) = overlays_in(&ctm, 1, Face::North, &[([-1, 0, 0], 1)]);
        assert_eq!((layers, pick), (vec![1001, 3000], Pick::Keep));
    }

    #[test]
    fn a_picked_tile_can_be_picked_again() {
        // b (rule 1) gives glass its tile b0, layer 2000; t (rule 0, a tile rule, tried first)
        // names that tile, so it draws in b0's place.
        let chain = |t: &str| {
            world(&[
                ("b.properties", "matchBlocks=glass\nmethod=fixed\ntiles=b0"),
                ("t.properties", t),
            ])
        };
        let ctm = chain("matchTiles=optifine/ctm/b0.png\nmethod=fixed\ntiles=0");
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(1000));
        // A chained rule keeping the tile (<default>) leaves b's.
        let ctm = chain("matchTiles=optifine/ctm/b0.png\nmethod=fixed\ntiles=<default>");
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
        // One that names another block doesn't chain on glass.
        let ctm = chain("matchTiles=optifine/ctm/b0.png\nmatchBlocks=stone\nmethod=fixed\ntiles=0");
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
        // An overlay tried on the face's own sprite and on the picked tile draws once.
        let ctm = chain("matchTiles=glass optifine/ctm/b0.png\nmethod=overlay_fixed\ntiles=0");
        let (overlays, pick) = overlays_in(&ctm, 1, Face::Up, &[]);
        assert_eq!((overlays, pick), (vec![1000], tile(2000)));
        // A rule after the one that picked, naming both the face's sprite and the tile, wasn't
        // tried: it chains.
        let ctm = world(&[
            ("a.properties", "matchTiles=glass\nmethod=fixed\ntiles=a0"),
            (
                "c.properties",
                "matchTiles=glass optifine/ctm/a0.png\nmethod=fixed\ntiles=0",
            ),
        ]);
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
    }

    #[test]
    fn orient_texture_follows_the_quads_uvs() {
        let rule = |orient: &str| {
            world(&[(
                "glass.properties",
                &*format!("matchBlocks=glass\nmethod=horizontal\ntiles=0-3\norient={orient}"),
            )])
        };
        // A mirrored north face (a vanilla pane's): its texture's right is east.
        let mirrored = || {
            Some(Axes {
                right: [1, 0, 0],
                up: [0, 1, 0],
            })
        };
        let east = |o: [i32; 3]| if o == [1, 0, 0] { 1 } else { 0 };
        let at = |ctm: &ConnectedTextures| {
            ctm.pick(
                1,
                3,
                Face::North,
                [0, 64, 0],
                east,
                || None,
                mirrored,
                |_| {},
            )
        };
        // Glass to the east is to the texture's right: the row's left end.
        assert_eq!(at(&rule("texture")), tile(1000));
        // By the cube's directions it is to the left: the row's right end.
        assert_eq!(at(&rule("none")), tile(1002));
        // A plain overlay keeps the standard directions: glass north of a top face is above it
        // (tile 15) whatever the quad's texture says.
        let overlay = world(&[(
            "a.properties",
            "matchBlocks=stone\nmethod=overlay\nconnectBlocks=glass\ntiles=0-16\norient=texture",
        )]);
        let turned = || {
            Some(Axes {
                right: [0, 0, 1],
                up: [1, 0, 0],
            })
        };
        let north = |o: [i32; 3]| if o == [0, 0, -1] { 1 } else { 0 };
        let mut got = Vec::new();
        overlay.pick(
            2,
            7,
            Face::Up,
            [0, 64, 0],
            north,
            || None,
            turned,
            |o| got.push(o.layer),
        );
        assert_eq!(got, [1015]);
    }

    #[test]
    fn stacked_panes_hide_the_strips_they_share() {
        let props = |pairs: &[(&str, &str)]| -> BlockProperties {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // 0 a pane with north and east arms, 1 the same pane with only a north arm, 2 another
        // pane block, 3 a pane no rule names.
        let states = [
            (
                "minecraft:glass_pane",
                props(&[("north", "true"), ("east", "true")]),
            ),
            ("minecraft:glass_pane", props(&[("north", "true")])),
            (
                "minecraft:white_stained_glass_pane",
                props(&[("north", "true")]),
            ),
            ("minecraft:iron_bars", props(&[("north", "true")])),
        ];
        let ctm = ConnectedTextures::build(
            rules(&[(
                "pane.properties",
                "matchBlocks=glass_pane white_stained_glass_pane\nmethod=ctm\ntiles=0-46",
            )]),
            states.iter().map(|(n, p)| Some((*n, p))),
            |_| None,
        );
        let (post, north, east) = ([0.5, 0.5], [0.5, 0.2], [0.8, 0.5]);
        assert!(ctm.stacked_pane_hides(0, 1, post));
        assert!(ctm.stacked_pane_hides(0, 1, north));
        // The pane above has no east arm: the strip shows; one with it hides it.
        assert!(!ctm.stacked_pane_hides(0, 1, east));
        assert!(ctm.stacked_pane_hides(1, 0, east));
        let (south, west) = ([0.5, 0.8], [0.2, 0.5]);
        assert!(!ctm.stacked_pane_hides(1, 0, south));
        assert!(!ctm.stacked_pane_hides(1, 0, west));
        assert!(ctm.stacked_pane_hides(1, 0, north));
        // Another block, or a pane without rules, hides nothing.
        assert!(!ctm.stacked_pane_hides(1, 2, post));
        assert!(!ctm.stacked_pane_hides(3, 3, post));
    }

    #[test]
    fn sprite_names() {
        assert_eq!(
            sprite_name("assets/minecraft/textures/block/dirt.png"),
            "minecraft:block/dirt"
        );
        assert_eq!(
            sprite_name("assets/minecraft/optifine/ctm/glass/0.png"),
            "assets/minecraft/optifine/ctm/glass/0.png"
        );
    }
}
