//! Where a block's textures are in the block sprite arrays, side by side, for drawing the block
//! somewhere the mesher does not: a native mod's own pass drawing blocks as voxels
//! (`native_block_sprites`, `WorldDrawn::block_sprites`; docs/native-mods.md). The answer is the
//! model's own faces, as the mesher bakes them: each face's sprite (its array and layer), where on
//! the block's side it lies and how its texture runs across it, and its tint source.
//!
//! Each sprite is a whole layer of a square texture array (`sprites::Sprites`), so a face's UVs run
//! 0 to 1 within its layer; the renderer's split (`Renderer::sprite_split`) layers to an array.
use crate::{mesher::TintSource, models::catalog::BakedFace, streaming::WorkerContext};
use horizon_assets::model::Direction;
use horizon_core::position::BlockPos;
use horizon_world::registry::{BlockRegistry, BlockStateId};

/// A sprite: its array (0 for every vanilla texture on most GPUs; past the first array's layers in
/// 1 to 7) and its layer in that array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteRef {
    pub array: u32,
    pub layer: u32,
    /// It animates (water, lava, fire, sea lanterns...): the arrays always hold the frame being shown.
    pub animated: bool,
    /// Some of its pixels are clear (alpha 0): leaves, glass, flowers. Draw those as holes.
    pub cutout: bool,
}

/// One face of a block's model on one of its sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceSprite {
    pub sprite: SpriteRef,
    /// Where the face lies on the side, in the side's two axes ([`side_axes`]), from 0 to 1 across the block:
    /// `[a0, b0, a1, b1]`. `[0, 0, 1, 1]` for a whole face.
    pub rect: [f32; 4],
    /// How deep into the block the face lies from the side's own plane (0 for a cube's face, 0.5 for a slab's top).
    pub inset: f32,
    /// The texture across the side: the point (a, b) on it shows `uv = [u0, v0] + a * [du_u, du_v] + b * [dv_u,
    /// dv_v]` of the sprite, `[u0, v0, du_u, du_v, dv_u, dv_v]`. Rotations and mirrors (a log on its side, a
    /// model's `rotation` and `uvlock`) are in it.
    pub uv: [f32; 6],
    /// Its colour source, none for an untinted face: multiply the texture by the source's colour.
    pub tint: Option<TintSource>,
    /// It has partly clear pixels: drawn sorted and blended.
    pub translucent: bool,
}

/// What a block draws on each side, in [`Direction::ALL`]'s order (down, up, north, south, west, east).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockSprites {
    /// The faces on each side, in the model's order: the one drawn first first (a grass block's side: its dirt
    /// and grass, then the grass overlay tinted over it). A side with none is empty. Faces that do not lie flat
    /// on a side (a flower's crossed planes, a rotated element) are left out.
    pub sides: [Vec<FaceSprite>; 6],
    /// The model's particle sprite (what breaking it scatters, a flower's whole picture), if it has one.
    pub particle: Option<SpriteRef>,
    /// The model is a plain cube: one whole face on each side, at its plane.
    pub cube: bool,
}

/// The two axes a side's `rect` and `uv` count along, as indices into (x, y, z): down and up across x then z; north
/// and south across x then y; west and east across z then y. (The block's corner at 0, 0, 0 is 0 on each.)
pub fn side_axes(side: Direction) -> (usize, usize) {
    match side {
        Direction::Down | Direction::Up => (0, 2),
        Direction::North | Direction::South => (0, 1),
        Direction::West | Direction::East => (2, 1),
    }
}

/// The axis a side faces along, and where its plane is (0 or 1).
fn side_plane(side: Direction) -> (usize, f32) {
    match side {
        Direction::Down => (1, 0.0),
        Direction::Up => (1, 1.0),
        Direction::North => (2, 0.0),
        Direction::South => (2, 1.0),
        Direction::West => (0, 0.0),
        Direction::East => (0, 1.0),
    }
}

/// A block state from `name[property=value,...]`, the namespace `minecraft:` when it has none; properties left
/// out take the block's default. None for a block or a property the game does not know.
pub fn parse_state(registry: &BlockRegistry, text: &str) -> Option<BlockStateId> {
    let (name, props) = match text.trim().split_once('[') {
        Some((n, rest)) => (n, rest.strip_suffix(']')?),
        None => (text.trim(), ""),
    };
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    let default = registry.default_state(&name)?;
    let mut state = registry.state(default)?.clone();
    for pair in props.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=')?;
        let slot = state.properties.get_mut(k.trim())?;
        *slot = v.trim().to_owned();
    }
    registry.resolve(&state).ok()
}

/// The sprites `context`'s resources hold, `split` layers to an array (`Renderer::sprite_split`).
pub struct Lookup<'a> {
    context: &'a WorkerContext,
    split: u32,
}

impl<'a> Lookup<'a> {
    pub fn new(context: &'a WorkerContext, split: u32) -> Self {
        Self {
            context,
            split: split.max(1),
        }
    }
    /// The sprite at `layer` of all the layers.
    pub fn sprite(&self, layer: u32) -> SpriteRef {
        let render = &self.context.render;
        SpriteRef {
            array: layer / self.split,
            layer: layer % self.split,
            animated: render.animations.iter().any(|a| a.layer == layer),
            cutout: render
                .sprite_mips
                .get(layer as usize)
                .is_some_and(|m| m.has_transparent),
        }
    }

    /// A texture by name (`minecraft:block/stone`, `block/stone`); none for one no model uses (the arrays hold only
    /// the textures models and the game's own drawing name).
    pub fn texture(&self, name: &str) -> Option<SpriteRef> {
        let layers = &self.context.render.sprite_layers;
        let layer = layers
            .get(name)
            .or_else(|| layers.get(&format!("minecraft:{name}")))?;
        Some(self.sprite(*layer))
    }

    /// What block `name` (`minecraft:oak_log[axis=x]`; properties left out take the default) draws on each side;
    /// none for a block the game does not know.
    pub fn block(&self, name: &str) -> Option<BlockSprites> {
        let state = parse_state(self.context.assets.registry, name)?;
        Some(self.state(state))
    }

    /// What `state` draws on each side. A fluid has its still texture on all six; a block drawn only as a block
    /// entity (a chest, a sign) or not at all (air) has empty sides, and its particle sprite if any.
    pub fn state(&self, state: BlockStateId) -> BlockSprites {
        let render = &self.context.render;
        let catalog = &render.model_catalog;
        let mut sides: [Vec<FaceSprite>; 6] = Default::default();
        let block = self
            .context
            .assets
            .registry
            .state(state)
            .map_or("", |s| s.name.as_str());
        let fluid = match block {
            "minecraft:water" | "minecraft:bubble_column" => {
                Some(("block/water_still", Some(TintSource::Water), true))
            }
            "minecraft:lava" => Some(("block/lava_still", None, false)),
            _ => None,
        };
        if let Some((texture, tint, translucent)) = fluid {
            let sprite = self.texture(texture);
            if let Some(sprite) = sprite {
                for side in &mut sides {
                    side.push(FaceSprite {
                        sprite,
                        rect: [0.0, 0.0, 1.0, 1.0],
                        inset: 0.0,
                        uv: [0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                        tint,
                        translucent,
                    });
                }
            }
            return BlockSprites {
                sides,
                particle: sprite,
                cube: sprite.is_some(),
            };
        }
        let models = catalog
            .collect(state, BlockPos::new(0, 0, 0))
            .unwrap_or_default();
        let mut all = 0;
        for face in models.iter().flat_map(|(_, m)| &m.faces) {
            all += 1;
            let Some((side, sprite)) = self.face(state, face) else {
                continue;
            };
            sides[side as usize].push(sprite);
        }
        let particle = catalog
            .particle(state)
            .or_else(|| {
                models
                    .iter()
                    .flat_map(|(_, m)| &m.faces)
                    .map(|f| f.layer)
                    .next()
            })
            .map(|layer| self.sprite(layer));
        // a cube: every side has a whole face at its plane (and perhaps overlays laid on it), and the model has no
        // other faces
        let whole = |f: &FaceSprite| f.rect == [0.0, 0.0, 1.0, 1.0] && f.inset == 0.0;
        let cube = all == sides.iter().map(Vec::len).sum::<usize>()
            && sides.iter().all(|s| !s.is_empty() && s.iter().all(whole));
        BlockSprites {
            sides,
            particle,
            cube,
        }
    }

    /// A model face as it lies on its side; none when it does not lie flat on one.
    fn face(&self, state: BlockStateId, face: &BakedFace) -> Option<(Direction, FaceSprite)> {
        let q = &face.quad;
        let side = q.facing;
        let (normal, plane) = side_plane(side);
        let depth = q.positions[0][normal];
        if q.positions.iter().any(|p| (p[normal] - depth).abs() > 1e-4) {
            return None;
        }
        let (a, b) = side_axes(side);
        let ab: [[f32; 2]; 4] = q.positions.map(|p| [p[a], p[b]]);
        let uv = affine(&ab, &q.uv)?;
        let (mut a0, mut b0, mut a1, mut b1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [x, y] in ab {
            a0 = a0.min(x);
            b0 = b0.min(y);
            a1 = a1.max(x);
            b1 = b1.max(y);
        }
        let snap = |v: f32| {
            let r = (v * 4096.0).round() / 4096.0;
            if r == 0.0 { 0.0 } else { r }
        };
        Some((
            side,
            FaceSprite {
                sprite: self.sprite(face.layer),
                rect: [a0, b0, a1, b1].map(snap),
                inset: snap((depth - plane).abs()),
                uv: uv.map(snap),
                tint: self.context.tables.tint_source(state, q.tint_index),
                translucent: face.translucent,
            },
        ))
    }
}

/// The affine map from a face's points on its side to its UVs, `[u0, v0, du_u, du_v, dv_u, dv_v]`, from three of
/// its corners that are not in a line (a model's faces are parallelograms on their side).
fn affine(ab: &[[f32; 2]; 4], uv: &[[f32; 2]; 4]) -> Option<[f32; 6]> {
    for [i, j, k] in [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]] {
        let d1 = [ab[j][0] - ab[i][0], ab[j][1] - ab[i][1]];
        let d2 = [ab[k][0] - ab[i][0], ab[k][1] - ab[i][1]];
        let det = d1[0] * d2[1] - d1[1] * d2[0];
        if det.abs() < 1e-6 {
            continue;
        }
        let e1 = [uv[j][0] - uv[i][0], uv[j][1] - uv[i][1]];
        let e2 = [uv[k][0] - uv[i][0], uv[k][1] - uv[i][1]];
        // e1 = d1.a * du + d1.b * dv, e2 = d2.a * du + d2.b * dv
        let du = [
            (e1[0] * d2[1] - e2[0] * d1[1]) / det,
            (e1[1] * d2[1] - e2[1] * d1[1]) / det,
        ];
        let dv = [
            (e2[0] * d1[0] - e1[0] * d2[0]) / det,
            (e2[1] * d1[0] - e1[1] * d2[0]) / det,
        ];
        let o = [
            uv[i][0] - ab[i][0] * du[0] - ab[i][1] * dv[0],
            uv[i][1] - ab[i][0] * du[1] - ab[i][1] * dv[1],
        ];
        return Some([o[0], o[1], du[0], du[1], dv[0], dv[1]]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_faces_texture_runs_across_its_side() {
        // a whole face, the texture upright: u along a, v along b
        let ab = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(
            affine(&ab, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
            Some([0.0, 0.0, 1.0, 0.0, 0.0, 1.0])
        );
        // turned a quarter (a log on its side): u runs down b, v back along a
        let turned = affine(&ab, &[[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]).unwrap();
        assert_eq!(turned, [0.0, 1.0, 0.0, -1.0, 1.0, 0.0]);
        // a slab's side: the lower half of the block, the lower half of the texture (v counts down)
        let half = [[0.0, 0.0], [1.0, 0.0], [1.0, 0.5], [0.0, 0.5]];
        let m = affine(&half, &[[0.0, 1.0], [1.0, 1.0], [1.0, 0.5], [0.0, 0.5]]).unwrap();
        assert_eq!(m, [0.0, 1.0, 1.0, 0.0, 0.0, -1.0]);
        // a face seen edge on has no map
        assert_eq!(affine(&[[0.0, 0.0]; 4], &[[0.0, 0.0]; 4]), None);
    }

    /// The game's own blocks from real prepared resources: a cube, a grass block's tinted top and overlaid sides, a
    /// log on its side (its texture turned), leaves (cut out, foliage), water, a slab and a flower.
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn real_blocks_name_their_sprites() -> crate::Result<()> {
        use crate::{
            assets::Assets, biome::Biomes, blocks::BlockFacts, light::LightTables,
            mesher::MeshTables,
        };
        use std::sync::Arc;
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let render = Arc::new(assets.reload(&root)?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let tables = Arc::new(MeshTables::new(
            &assets,
            &render,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
        )?);
        let context = WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets: assets.clone(),
            render,
            biomes: Arc::new(Biomes::load(&root)?),
        };
        let lookup = Lookup::new(&context, 2048);
        let whole = [0.0, 0.0, 1.0, 1.0];
        let upright = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];

        let stone = lookup.block("minecraft:stone").unwrap();
        assert!(stone.cube);
        let top = stone.sides[Direction::Up as usize][0];
        assert_eq!(
            (top.rect, top.inset, top.tint, top.sprite.array),
            (whole, 0.0, None, 0)
        );
        assert_eq!(lookup.texture("minecraft:block/stone"), Some(top.sprite));
        assert_eq!(lookup.texture("block/stone"), Some(top.sprite));
        assert!(!top.sprite.animated && !top.sprite.cutout);

        let grass = lookup.block("grass_block").unwrap();
        assert!(grass.cube);
        assert_eq!(
            grass.sides[Direction::Up as usize][0].tint,
            Some(TintSource::Grass)
        );
        assert_eq!(grass.sides[Direction::Down as usize][0].tint, None);
        let side = &grass.sides[Direction::North as usize];
        assert_eq!(side.len(), 2, "the side and its overlay");
        assert_eq!(
            (side[0].tint, side[1].tint),
            (None, Some(TintSource::Grass))
        );
        assert_eq!(
            lookup.texture("block/grass_block_side_overlay"),
            Some(side[1].sprite)
        );

        // seen from outside, a side's texture runs from its top left: on the north side (looking south, east on
        // the left) u falls with x and v with y; on top, u and v run with x and z (stone's first variant is turned)
        let planks = lookup.block("oak_planks").unwrap();
        let uv = |b: &BlockSprites, d: Direction| b.sides[d as usize][0].uv;
        let north = uv(&planks, Direction::North);
        assert_eq!(north, [1.0, 1.0, -1.0, 0.0, 0.0, -1.0]);
        assert_eq!(uv(&planks, Direction::Up), upright);
        assert_eq!(
            uv(&planks, Direction::South),
            [0.0, 1.0, 1.0, 0.0, 0.0, -1.0]
        );
        assert_eq!(
            uv(&planks, Direction::Down),
            [0.0, 1.0, 1.0, 0.0, 0.0, -1.0]
        );
        // an upright log's side runs as stone's; on its side (axis x) the texture is turned a quarter
        let log = lookup.block("oak_log").unwrap();
        let log_x = lookup.block("minecraft:oak_log[axis=x]").unwrap();
        assert_eq!(log.sides[Direction::North as usize][0].uv, north);
        let turned = log_x.sides[Direction::North as usize][0].uv;
        assert_eq!(
            (turned[2], turned[5]),
            (0.0, 0.0),
            "u runs along y, v along x: {turned:?}"
        );
        assert_eq!(
            log_x.sides[Direction::West as usize][0].sprite,
            log.sides[Direction::Up as usize][0].sprite,
            "its end faces west"
        );

        let leaves = lookup.block("oak_leaves").unwrap();
        let leaf = leaves.sides[Direction::Up as usize][0];
        assert!(leaf.sprite.cutout);
        assert_eq!(leaf.tint, Some(TintSource::Foliage));
        assert!(matches!(
            lookup.block("spruce_leaves").unwrap().sides[0][0].tint,
            Some(TintSource::Fixed(0x61_9961))
        ));

        let water = lookup.block("water").unwrap();
        let w = water.sides[Direction::Up as usize][0];
        assert!(w.sprite.animated && w.translucent);
        assert_eq!(w.tint, Some(TintSource::Water));

        let slab = lookup.block("stone_slab").unwrap();
        assert!(!slab.cube);
        assert_eq!(slab.sides[Direction::Up as usize][0].inset, 0.5);
        assert_eq!(
            slab.sides[Direction::North as usize][0].rect,
            [0.0, 0.0, 1.0, 0.5]
        );

        let flower = lookup.block("dandelion").unwrap();
        assert!(!flower.cube && flower.sides.iter().all(Vec::is_empty));
        assert!(flower.particle.is_some_and(|p| p.cutout));

        assert_eq!(lookup.block("minecraft:no_such_block"), None);
        assert_eq!(lookup.block("stone[no_such=1]"), None);
        assert!(lookup.block("air").unwrap().sides.iter().all(Vec::is_empty));
        Ok(())
    }
}
