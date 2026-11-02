//! Fusion connected textures at mesh time. A pack's connecting textures (a `fusion` section in
//! their `.mcmeta`) are cut into a sprite per tile at load, and every baked face of a
//! `fusion:connecting` model that shows one is given the rule it joins by: the sheet and its
//! predicates, with their blocks resolved to state ids. Meshing such a face evaluates them in
//! the face's eight directions and draws the layout's tiles through [`crate::connected`]'s
//! [`Pick`] and [`Overlay`]. Without Fusion files nothing is loaded and nothing changes.
//!
//! Directions follow a cube's default UVs ([`Axes::cube`]): the top and bottom of a model
//! turned about x, or turned with `uvlock`, join a quarter turn off.
//!
//! The format is `horizon_optifine::fusion`'s.

use crate::connected::{Overlay, Pick, TileLayer};
use crate::models::StateModels;
use crate::sprites::Sprites;
use horizon_optifine::cem::json::Json;
use horizon_optifine::ctm::connect::{self, Axes, Face};
use horizon_optifine::fusion::{
    Connection, Layout, Tiles,
    model::{Connections, ModelType},
    predicate::Query,
    texture::{Kind, Meta},
};
use horizon_world::block_state_properties::BlockProperties;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{collections::HashMap, fs, num::NonZeroU16, path::Path};

/// A connecting texture as drawn: its layout and its tiles, `None` for an empty one.
struct Sheet {
    layout: Layout,
    tiles: Vec<Option<TileLayer>>,
}

/// A sheet and what it joins by, blocks as sorted state ids.
struct Rule {
    sheet: usize,
    connection: Connection<Vec<u32>>,
}

#[derive(Default)]
pub struct FusionTextures {
    sheets: Vec<Sheet>,
    /// By [`crate::models::BakedFace::fusion`].
    rules: Vec<Rule>,
    /// Each state's block, for `is_same_block` (`u32::MAX` for an id the registry skips).
    blocks: Vec<u32>,
}

/// A model's reference to a texture or another model (`block/glass`), namespaced.
fn namespaced(reference: &str) -> String {
    horizon_optifine::matchers::namespaced(reference)
}

fn read_model(root: &Path, name: &str) -> Option<Json> {
    let (ns, path) = name.split_once(':')?;
    let text = fs::read_to_string(root.join(format!("assets/{ns}/models/{path}.json"))).ok()?;
    Json::parse(&text).ok()
}

/// What a model's face draws and joins by: its sprite, its side in the model, and its
/// predicates (`None`: the texture's own).
type ModelFace = (String, Face, Option<Connection>);

/// Of a `fusion:connecting` model: each face's [`ModelFace`]. Empty for any other model.
fn connecting_model(root: &Path, name: &str) -> Vec<ModelFace> {
    let Some(model) = read_model(root, name) else {
        return Vec::new();
    };
    if ModelType::of(&model) != Some(ModelType::Connecting) {
        return Vec::new();
    }
    // Its parents, up to one without a file (`builtin/generated`); deeper than vanilla's few
    // levels is a loop.
    let mut chain = vec![model];
    while chain.len() < 16
        && let Some(parent) = chain.last().and_then(|m| m.get("parent")?.as_str())
        && let Some(parent) = read_model(root, &namespaced(parent))
    {
        chain.push(parent);
    }
    // Parents first, so a child's keys replace theirs.
    let mut connections = Connections::Textures;
    let mut textures: HashMap<&str, &str> = HashMap::new();
    for model in chain.iter().rev() {
        match Connections::of(model) {
            Ok(own) => connections = own.inherit(&connections),
            Err(e) => log::warn!("Horizon: Fusion model {name}: {e}"),
        }
        for (key, value) in model
            .get("textures")
            .and_then(Json::as_object)
            .unwrap_or(&[])
        {
            // A sprite, or an object naming one (`{"sprite": ..., "force_translucent": true}`).
            let value = value.as_str().or_else(|| value.get("sprite")?.as_str());
            if let Some(value) = value {
                textures.insert(key.as_str(), value);
            }
        }
    }
    // A `#key` as vanilla resolves it.
    let resolve = |reference: &str| {
        let mut value = reference;
        for _ in 0..16 {
            match value.strip_prefix('#') {
                Some(key) => value = textures.get(key)?,
                None => return Some(namespaced(value)),
            }
        }
        None
    };
    // The elements are the nearest model's that has some.
    let elements = chain
        .iter()
        .find_map(|m| m.get("elements")?.as_array())
        .unwrap_or(&[]);
    let mut out: Vec<ModelFace> = Vec::new();
    let faces = elements
        .iter()
        .filter_map(|e| e.get("faces")?.as_object())
        .flatten();
    for (side, face) in faces {
        let side = match side.as_str() {
            "down" => Face::Down,
            "up" => Face::Up,
            "north" => Face::North,
            "south" => Face::South,
            "west" => Face::West,
            "east" => Face::East,
            _ => continue,
        };
        let Some(texture) = face.get("texture").and_then(Json::as_str) else {
            continue;
        };
        let Some(sprite) = resolve(texture) else {
            continue;
        };
        if !out.iter().any(|(s, f, _)| *s == sprite && *f == side) {
            let key = face.get("connections").and_then(Json::as_str);
            out.push((sprite, side, connections.for_face(key, texture).cloned()));
        }
    }
    out
}

/// Where a model's `face` points once a blockstate turns it `x`, then `y` quarter turns.
fn turned(face: Face, [x, y, _]: [u8; 3]) -> Face {
    let mut n = face.normal();
    for _ in 0..x % 4 {
        // x=90 takes up to north.
        n = [n[0], n[2], -n[1]];
    }
    for _ in 0..y % 4 {
        // y=90 takes north to east.
        n = [-n[2], n[1], n[0]];
    }
    Face::ALL
        .into_iter()
        .find(|f| f.normal() == n)
        .expect("a quarter turn maps faces onto faces")
}

/// The connecting textures under `root`, by sprite name.
fn connecting_textures(root: &Path) -> HashMap<String, Meta> {
    let mut out = HashMap::new();
    let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
        return out;
    };
    for ns in namespaces.flatten() {
        let ns_name = ns.file_name().to_string_lossy().into_owned();
        let textures = ns.path().join("textures");
        let mut dirs = vec![textures.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                // Linked folders too: mods' content links into the packs' tree.
                let dir = match entry.file_type() {
                    Ok(t) if t.is_symlink() => path.is_dir(),
                    Ok(t) => t.is_dir(),
                    Err(_) => path.is_dir(),
                };
                if dir {
                    dirs.push(path);
                    continue;
                }
                let Some(png) = path.to_str().and_then(|p| p.strip_suffix(".png.mcmeta")) else {
                    continue;
                };
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                if !text.contains("fusion") {
                    continue;
                }
                let Some(rel) = Path::new(png)
                    .strip_prefix(&textures)
                    .ok()
                    .and_then(|p| p.to_str())
                else {
                    continue;
                };
                let name = format!("{ns_name}:{}", rel.replace('\\', "/"));
                match Meta::from_mcmeta(&text) {
                    Ok(Some(meta)) => {
                        out.insert(name, meta);
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("Horizon: Fusion texture {name}: {e}"),
                }
            }
        }
    }
    out
}

/// `sprites`' tiles of the connecting texture `name`, and the predicates it joins by itself.
fn sheet(
    root: &Path,
    sprites: &mut Sprites,
    name: &str,
    meta: Meta,
) -> Option<(Sheet, Connection)> {
    let Kind::Connecting {
        layout,
        connections,
        per_tile_animation,
        ..
    } = meta.kind
    else {
        return None;
    };
    let tiles = sprites
        .insert_tiles(root, name, layout.grid(), per_tile_animation)
        .map_err(|e| log::warn!("Horizon: Fusion texture {name}: {e}"))
        .ok()?;
    // A layer past the mesher's `u16` leaves the sheet to vanilla.
    let tiles = tiles
        .into_iter()
        .map(|t| match t {
            None => Some(None),
            Some(l) => Some(Some(TileLayer::Layer {
                layer: u16::try_from(l).ok()?,
                translucent: sprites.translucent[l as usize],
                opaque: sprites.opaque[l as usize],
            })),
        })
        .collect::<Option<Vec<_>>>()?;
    Some((Sheet { layout, tiles }, connections))
}

impl FusionTextures {
    /// The connecting textures under `root`: the tiles of those `fusion:connecting` models
    /// draw are inserted into `sprites` (before it finishes), and each face in `catalog` that
    /// shows one gets its rule.
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
        catalog: &mut StateModels,
    ) -> Self {
        let mut textures = connecting_textures(root);
        if textures.is_empty() {
            return Self::default();
        }
        // The layers models show the sheets as: only their models are read.
        let shown: std::collections::HashSet<u32> = textures
            .keys()
            .filter_map(|name| sprites.layers.get(name).copied())
            .collect();
        let mut models: HashMap<String, Vec<ModelFace>> = HashMap::new();
        let mut sheets: Vec<(Sheet, Connection)> = Vec::new();
        let mut by_name: HashMap<String, Option<usize>> = HashMap::new();
        let mut rules: Vec<(usize, Connection)> = Vec::new();
        for (variant, model) in catalog.models_mut() {
            if !model.faces.iter().any(|f| shown.contains(&f.layer)) {
                continue;
            }
            let found = models
                .entry(namespaced(&variant.model))
                .or_insert_with_key(|name| connecting_model(root, name));
            for face in &mut model.faces {
                let sprite = &face.quad.material.sprite;
                // The model's face on the side this one turned to, else any with its sprite.
                let side = Face::ALL[face.quad.facing as usize];
                let mut drawing = found.iter().filter(|(s, _, _)| s == sprite);
                let Some((_, _, connection)) = drawing
                    .clone()
                    .find(|(_, f, _)| turned(*f, variant.rotations) == side)
                    .or_else(|| drawing.next())
                else {
                    continue;
                };
                // The sheet, cut into tiles on first use.
                let sheet = *by_name.entry(sprite.clone()).or_insert_with(|| {
                    let meta = textures.remove(sprite)?;
                    sheets.push(sheet(root, sprites, sprite, meta)?);
                    Some(sheets.len() - 1)
                });
                let Some(sheet) = sheet else {
                    continue;
                };
                let connection = connection
                    .clone()
                    .unwrap_or_else(|| sheets[sheet].1.clone());
                let rule = match rules.iter().position(|r| r.0 == sheet && r.1 == connection) {
                    Some(rule) => rule,
                    None => {
                        rules.push((sheet, connection));
                        rules.len() - 1
                    }
                };
                face.fusion = u16::try_from(rule + 1).ok().and_then(NonZeroU16::new);
            }
        }
        let states = (0..registry.len() as u32).map(|id| {
            registry
                .state(BlockStateId(id))
                .map(|s| (s.name.as_str(), &s.properties))
        });
        let sheets = sheets.into_iter().map(|(sheet, _)| sheet).collect();
        Self::build(sheets, rules, states)
    }

    /// The lookups for `rules` over every block state (`None` for an id the registry skips).
    fn build<'a>(
        sheets: Vec<Sheet>,
        rules: Vec<(usize, Connection)>,
        states: impl Iterator<Item = Option<(&'a str, &'a BlockProperties)>>,
    ) -> Self {
        let states: Vec<_> = states.collect();
        let mut block_states: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut names = HashMap::new();
        let blocks = (0..states.len() as u32)
            .map(|id| match states[id as usize] {
                Some((name, _)) => {
                    block_states.entry(name).or_default().push(id);
                    let next = names.len() as u32;
                    *names.entry(name).or_insert(next)
                }
                None => u32::MAX,
            })
            .collect();
        // Block lists to the states they match.
        let mut resolve = |list: Vec<horizon_optifine::BlockMatch>| {
            let mut ids: Vec<u32> = list
                .iter()
                .flat_map(|m| {
                    let candidates = block_states
                        .get(m.id.as_str())
                        .map_or(&[][..], Vec::as_slice);
                    candidates.iter().copied().filter(|&id| {
                        states[id as usize].is_some_and(|(name, props)| {
                            m.matches(name, |k| props.get(k).map(String::as_str))
                        })
                    })
                })
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        let rules = rules
            .into_iter()
            .map(|(sheet, connection)| Rule {
                sheet,
                connection: connection.map_blocks(&mut resolve),
            })
            .collect();
        Self {
            sheets,
            rules,
            blocks,
        }
    }

    /// What the `face` of the block `state` draws for a quad whose rule is `rule` and whose
    /// tint index is `tint`. `neighbour` answers the state at an offset from the block,
    /// `covers` whether a state hides the face of a block beside it; the overlay layout's
    /// pieces, tinted as the quad, go to `overlay`.
    #[allow(clippy::too_many_arguments)]
    pub fn pick(
        &self,
        rule: NonZeroU16,
        state: u32,
        face: Face,
        tint: i32,
        neighbour: impl Fn([i32; 3]) -> u32,
        covers: impl Fn(u32) -> bool,
        mut overlay: impl FnMut(Overlay),
    ) -> Option<Pick> {
        let rule = self.rules.get(usize::from(rule.get() - 1))?;
        let normal = face.normal();
        let offsets = connect::offsets(Axes::cube(face));
        let block = self.blocks.get(state as usize).filter(|&&b| b != u32::MAX);
        let mask = rule.connection.mask(|bit, query| {
            let &(_, [x, y, z]) = offsets.iter().find(|o| o.0 == bit).expect("a mask bit");
            let in_front = || neighbour([x + normal[0], y + normal[1], z + normal[2]]);
            match query {
                Query::SameBlock => {
                    block.is_some() && self.blocks.get(neighbour([x, y, z]) as usize) == block
                }
                Query::SameState => neighbour([x, y, z]) == state,
                Query::Neighbour(ids) => ids.binary_search(&neighbour([x, y, z])).is_ok(),
                Query::InFront(ids) => ids.binary_search(&in_front()).is_ok(),
                Query::FaceVisible => !covers(in_front()),
            }
        });
        let sheet = &self.sheets[rule.sheet];
        let tile = |i: u8| sheet.tiles.get(usize::from(i)).copied().flatten();
        // An empty tile draws nothing: a pack leaves empty only the tiles its layout's uses
        // never need.
        Some(match sheet.layout.tiles(mask) {
            Tiles::One(t) => tile(t).map_or(Pick::Nothing, Pick::Tile),
            Tiles::Quarters(q) => match q.map(tile) {
                [Some(a), Some(b), Some(c), Some(d)] => Pick::Quarters([a, b, c, d]),
                _ => Pick::Nothing,
            },
            Tiles::Overlay(mask) => {
                for t in Layout::overlay_tiles(mask) {
                    if let Some(TileLayer::Layer {
                        layer, translucent, ..
                    }) = tile(t)
                    {
                        overlay(Overlay {
                            layer,
                            translucent,
                            tint: (tint >= 0).then_some((tint, None)),
                        });
                    }
                }
                Pick::Nothing
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tile `i` of a sheet is layer `base + i`; `empty` tiles have none.
    fn sheet(layout: Layout, base: u16, empty: &[u16]) -> Sheet {
        let (w, h) = layout.grid();
        Sheet {
            layout,
            tiles: (0..(w * h) as u16)
                .map(|i| (!empty.contains(&i)).then_some(layer(base + i)))
                .collect(),
        }
    }

    fn predicate(json: &str) -> Connection {
        Connection::parse(&Json::parse(json).unwrap()).unwrap()
    }

    fn layer(layer: u16) -> TileLayer {
        TileLayer::Layer {
            layer,
            translucent: false,
            opaque: false,
        }
    }

    /// The lookups over blocks named `names` (a state each).
    fn world(
        names: &[&str],
        sheets: Vec<Sheet>,
        rules: Vec<(usize, Connection)>,
    ) -> FusionTextures {
        let none = BlockProperties::default();
        let states = names.iter().map(|n| Some((*n, &none)));
        FusionTextures::build(sheets, rules, states)
    }

    /// `pick` for the block at the origin among `blocks` (offset, state), the opaque ones in
    /// `opaque`; with the overlays it hands out.
    fn pick_in(
        fusion: &FusionTextures,
        rule: u16,
        state: u32,
        face: Face,
        blocks: &[([i32; 3], u32)],
        opaque: &[u32],
    ) -> (Option<Pick>, Vec<u16>) {
        // A face's rule counts from 1.
        let rule = NonZeroU16::new(rule + 1).unwrap();
        let at = |o: [i32; 3]| blocks.iter().find(|(p, _)| *p == o).map_or(0, |b| b.1);
        let mut overlays = Vec::new();
        let pick = fusion.pick(
            rule,
            state,
            face,
            -1,
            at,
            |s| opaque.contains(&s),
            |o| overlays.push(o.layer),
        );
        (pick, overlays)
    }

    #[test]
    fn glass_pieces_join_the_same_block() {
        // Glass twice (two states of one block), then stone.
        let fusion = world(
            &[
                "minecraft:air",
                "minecraft:glass",
                "minecraft:glass",
                "minecraft:stone",
            ],
            vec![sheet(Layout::Pieced, 100, &[])],
            vec![(0, Connection::SameBlock)],
        );
        let at = |blocks: &[([i32; 3], u32)]| pick_in(&fusion, 0, 1, Face::North, blocks, &[]).0;
        // Alone: every quarter the lone tile.
        assert_eq!(at(&[]), Some(Pick::Quarters([layer(100); 4])));
        // A north face's right is west: glass there (in its other state) joins the right
        // quarters, the horizontal piece. Stone doesn't.
        let right = Pick::Quarters([layer(100), layer(103), layer(100), layer(103)]);
        assert_eq!(at(&[([-1, 0, 0], 2)]), Some(right));
        assert_eq!(
            at(&[([-1, 0, 0], 3)]),
            Some(Pick::Quarters([layer(100); 4]))
        );
        // A rule that doesn't exist draws as vanilla.
        assert_eq!(pick_in(&fusion, 1, 1, Face::North, &[], &[]).0, None);
    }

    #[test]
    fn overlays_reach_toward_visible_sand() {
        let sand = predicate(
            r#"{"type": "and", "predicates": [{"type": "match_block", "block": "sand"}, {"type": "is_face_visible"}]}"#,
        );
        let fusion = world(
            &["minecraft:air", "minecraft:stone", "minecraft:sand"],
            vec![sheet(Layout::Overlay, 200, &[7])],
            vec![(0, sand)],
        );
        // Sand to the north of stone's top face, its own top open: the top edge piece, (1, 2),
        // over a face that draws nothing itself.
        let north = [([0, 0, -1], 2)];
        assert_eq!(
            pick_in(&fusion, 0, 1, Face::Up, &north, &[]),
            (Some(Pick::Nothing), vec![213])
        );
        // Stone over that sand hides its top: no edge.
        let covered = [([0, 0, -1], 2), ([0, 1, -1], 1)];
        assert_eq!(
            pick_in(&fusion, 0, 1, Face::Up, &covered, &[1]),
            (Some(Pick::Nothing), vec![])
        );
        // Sand all round: the ring, (4, 1).
        let ring: Vec<_> = [-1, 0, 1]
            .into_iter()
            .flat_map(|x| [-1, 0, 1].map(|z| ([x, 0, z], 2)))
            .filter(|(o, _)| *o != [0, 0, 0])
            .collect();
        assert_eq!(pick_in(&fusion, 0, 1, Face::Up, &ring, &[]).1, [210]);
        // Its pieces take the quad's tint index.
        let mut tints = Vec::new();
        let at = |o: [i32; 3]| if o == [0, 0, -1] { 2 } else { 0 };
        let rule = NonZeroU16::new(1).unwrap();
        fusion.pick(rule, 1, Face::Up, 0, at, |_| false, |o| tints.push(o.tint));
        assert_eq!(tints, [Some((0, None))]);
    }

    #[test]
    fn in_front_and_empty_tiles() {
        // Dirt's top joins where sand sits on the block beside it; its full sheet leaves the
        // lone tile empty.
        let front = predicate(r#"{"type": "match_block_in_front", "block": "sand"}"#);
        let fusion = world(
            &["minecraft:air", "minecraft:dirt", "minecraft:sand"],
            vec![sheet(Layout::Full, 300, &[0])],
            vec![(0, front)],
        );
        // Sand on the block east of the top face: joined right, the full template's (1, 0).
        assert_eq!(
            pick_in(&fusion, 0, 1, Face::Up, &[([1, 1, 0], 2)], &[]).0,
            Some(Pick::Tile(layer(301)))
        );
        // Sand beside it, not in front of anything: alone, and the lone tile is empty.
        assert_eq!(
            pick_in(&fusion, 0, 1, Face::Up, &[([1, 0, 0], 2)], &[]).0,
            Some(Pick::Nothing)
        );
    }

    #[test]
    fn models_name_the_predicates_their_faces_draw() {
        let root =
            std::env::temp_dir().join(format!("horizon-fusion-models-{}", std::process::id()));
        let models = root.join("assets/demo/models/block");
        fs::create_dir_all(&models).unwrap();
        // A pane arm over a template whose `particle` names the pane sheet but draws no face
        // with it (as Fusion Connected Glass's posts do): only the faces count.
        fs::write(
            models.join("template.json"),
            r##"{"textures": {"particle": "#pane"}, "elements": [{"from": [7, 0, 0], "to": [9, 16, 7],
                "faces": {"up": {"texture": "#edge"}, "west": {"texture": "#pane"}}}]}"##,
        )
        .unwrap();
        fs::write(
            models.join("arm.json"),
            r#"{"loader": "fusion:model", "type": "fusion:connecting", "parent": "demo:block/template",
                "connections": {"edge": {"type": "true"}},
                "textures": {"pane": "demo:block/glass", "edge": {"sprite": "demo:block/edge"}}}"#,
        )
        .unwrap();
        fs::write(
            models.join("plain.json"),
            r#"{"parent": "demo:block/template", "textures": {"pane": "demo:block/glass"}}"#,
        )
        .unwrap();
        let arm = connecting_model(&root, "demo:block/arm");
        let plain = connecting_model(&root, "demo:block/plain");
        fs::remove_dir_all(&root).unwrap();
        // `edge` has the model's predicate; `pane` none, so the texture's own.
        assert_eq!(
            arm,
            [
                (
                    "demo:block/edge".to_owned(),
                    Face::Up,
                    Some(Connection::Const(true))
                ),
                ("demo:block/glass".to_owned(), Face::West, None),
            ]
        );
        // Not a Fusion model: vanilla.
        assert!(plain.is_empty());
    }

    #[test]
    fn faces_of_one_sprite_join_by_their_own_keys() {
        let root = std::env::temp_dir().join(format!("horizon-fusion-keys-{}", std::process::id()));
        let models = root.join("assets/demo/models/block");
        fs::create_dir_all(&models).unwrap();
        // The wiki's example: four keys on one texture, two of them joining by `blue`.
        fs::write(
            models.join("tiles.json"),
            r##"{"loader": "fusion:model", "type": "connecting",
                "textures": {"ice": "block/oak_tiles", "dirt": "block/oak_tiles", "stone": "block/oak_tiles"},
                "connections": {"blue": {"type": "match_block", "block": "ice"},
                    "dirt": {"type": "is_same_block"}, "default": {"type": "is_same_state"}},
                "elements": [{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {
                    "up": {"texture": "#ice", "connections": "#blue"},
                    "north": {"texture": "#dirt"}, "south": {"texture": "#stone"}}}]}"##,
        )
        .unwrap();
        let faces = connecting_model(&root, "demo:block/tiles");
        fs::remove_dir_all(&root).unwrap();
        let joins = |side| faces.iter().find(|f| f.1 == side).and_then(|f| f.2.clone());
        assert!(matches!(joins(Face::Up), Some(Connection::Neighbour(_))));
        assert_eq!(joins(Face::North), Some(Connection::SameBlock));
        assert_eq!(joins(Face::South), Some(Connection::SameState));
        // A blockstate's turns move them: x=90 takes the top to the north, y=90 the north to
        // the east.
        assert_eq!(turned(Face::Up, [1, 0, 0]), Face::North);
        assert_eq!(turned(Face::North, [0, 1, 0]), Face::East);
        assert_eq!(turned(Face::Up, [1, 1, 0]), Face::East);
    }
}
