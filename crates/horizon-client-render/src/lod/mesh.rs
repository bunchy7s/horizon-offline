//! Exposed voxel-column faces, merged without changing their block-scale UVs.
use super::{Column, Span, Spans, TileKey};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    mesher::{OPAQUE_LAYER, SectionMesh, TerrainVertex, fluid_face},
    streaming::WorkerContext,
};
use horizon_assets::model::Direction;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use std::collections::BTreeMap;

const CELLS: usize = 16;

pub(super) struct EdgeColumn<'a> {
    pub start: i32,
    pub end: i32,
    pub column: &'a Column,
}

#[derive(Clone, Copy)]
enum Tint {
    None,
    Grass,
    Foliage,
    Dry,
    Water,
    Constant(u32),
}
#[derive(Clone, Copy)]
struct Style {
    layer: u16,
    tint: Tint,
    translucent: bool,
}
#[derive(Clone, Copy)]
struct Styles {
    faces: [Option<Style>; 2],
    fluid: u8,
    opaque: bool,
}
#[derive(Clone, Copy)]
struct Paint {
    layer: u16,
    light: u16,
    color: u32,
    translucent: bool,
}
#[derive(Clone, Copy)]
struct Material {
    faces: [Option<Paint>; 2],
    fluid: u8,
    opaque: bool,
}
#[derive(Default)]
pub(super) struct Mesher {
    styles: FxHashMap<(BlockStateId, u8), Styles>,
}
impl Mesher {
    pub(super) fn mesh(
        &mut self,
        key: TileKey,
        columns: &[&Column],
        covered: &FxHashSet<(i32, i32)>,
        covered_bounds: Option<[i32; 4]>,
        context: &WorkerContext,
        neighbors: &[Vec<EdgeColumn<'_>>; 4],
        finer: &[TileKey],
    ) -> SectionMesh {
        mesh_columns_with_edges(
            key,
            columns,
            covered,
            covered_bounds,
            neighbors,
            finer,
            |span, direction, at| self.material(span, direction, at, context),
        )
    }
    /// `at`: the block column the face covers, for the grass colours that
    /// change across a biome (swamps).
    fn material(
        &mut self,
        span: &Span,
        direction: Direction,
        at: [i32; 2],
        context: &WorkerContext,
    ) -> Material {
        let styles = *self
            .styles
            .entry((span.state, direction as u8))
            .or_insert_with(|| {
                let assets = &context.assets;
                let name = assets
                    .registry
                    .state(span.state)
                    .map_or("", |s| s.name.as_str());
                let fluid = match name {
                    "minecraft:water" | "minecraft:bubble_column" => 1,
                    "minecraft:lava" => 2,
                    _ => 0,
                };
                if fluid != 0 {
                    let sprite = if fluid == 1 {
                        "block/water_still"
                    } else {
                        "block/lava_still"
                    };
                    let render = &context.render;
                    let layer = render
                        .sprite_layers
                        .get(sprite)
                        .or_else(|| render.sprite_layers.get(&format!("minecraft:{sprite}")))
                        .copied()
                        .unwrap_or(0) as u16;
                    return Styles {
                        faces: [
                            Some(Style {
                                layer,
                                tint: if fluid == 1 { Tint::Water } else { Tint::None },
                                translucent: fluid == 1,
                            }),
                            None,
                        ],
                        fluid,
                        opaque: fluid == 2,
                    };
                }
                let leaves = name.ends_with("_leaves");
                let tint = match name.strip_prefix("minecraft:").unwrap_or(name) {
                    "spruce_leaves" => Tint::Constant(0x61_9961),
                    "birch_leaves" => Tint::Constant(0x80_A755),
                    // Cherry (and other pre-coloured) leaves: the texture is
                    // already pink; biome foliage tint turns a grove yellow (#4688).
                    "cherry_leaves"
                    | "azalea_leaves"
                    | "flowering_azalea_leaves"
                    | "pale_oak_leaves" => Tint::None,
                    "leaf_litter" => Tint::Dry,
                    _ if leaves => Tint::Foliage,
                    _ => Tint::Grass,
                };
                let models = context
                    .render
                    .model_catalog
                    .collect(span.state, BlockPos::new(0, 0, 0))
                    .ok();
                let opaque = leaves
                    || models
                        .iter()
                        .flatten()
                        .flat_map(|(_, model)| &model.faces)
                        .any(|face| !face.translucent);
                let mut matching = models
                    .iter()
                    .flatten()
                    .flat_map(|(_, model)| &model.faces)
                    .filter(|face| face.quad.facing == direction);
                let mut faces = [None; 2];
                for slot in &mut faces {
                    let Some(face) = matching.next() else { break };
                    *slot = Some(Style {
                        layer: face.layer as u16 | if leaves { OPAQUE_LAYER } else { 0 },
                        tint: if face.quad.tint_index >= 0 {
                            tint
                        } else {
                            Tint::None
                        },
                        translucent: face.translucent && !leaves,
                    });
                    // Grass's tinted side overlay must follow its untinted base.
                    if name != "minecraft:grass_block"
                        || matches!(direction, Direction::Up | Direction::Down)
                    {
                        break;
                    }
                }
                Styles {
                    opaque,
                    faces,
                    fluid,
                }
            });
        let biome = context.biomes.get(span.biome);
        let shade = match direction {
            Direction::Up => 1.0,
            Direction::Down => 0.5,
            Direction::North | Direction::South => 0.8,
            Direction::West | Direction::East => 0.6,
        };
        Material {
            faces: styles.faces.map(|face| {
                face.map(|face| {
                    let rgb = match face.tint {
                        Tint::None => 0xffffff,
                        Tint::Grass => {
                            context
                                .biomes
                                .grass(span.biome, f64::from(at[0]), f64::from(at[1]))
                        }
                        Tint::Foliage => biome.foliage,
                        Tint::Dry => biome.dry_foliage,
                        Tint::Water => biome.water,
                        Tint::Constant(color) => color,
                    };
                    let channel = |shift: u32| (((rgb >> shift) & 255u32) as f32 * shade) as u32;
                    Paint {
                        layer: face.layer,
                        light: span.light,
                        color: channel(16) | channel(8) << 8 | channel(0) << 16 | 0xff000000,
                        translucent: face.translucent,
                    }
                })
            }),
            fluid: styles.fluid,
            opaque: styles.opaque,
        }
    }
}

struct Segment<'a> {
    span: &'a Span,
    min: f32,
    max: f32,
    material: Material,
    top: bool,
    bottom: bool,
}
fn hides(neighbor: &Span, material: Material, current: &Span, current_material: Material) -> bool {
    material.opaque
        || neighbor.state == current.state
        || (material.fluid != 0 && material.fluid == current_material.fluid)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FaceKey {
    direction: Direction,
    plane: u32,
    overlay: u8,
    layer: u16,
    light: u16,
    color: u32,
    translucent: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
}
type Faces = BTreeMap<FaceKey, Vec<Rect>>;
fn add_face(faces: &mut Faces, direction: Direction, plane: f32, rect: Rect, material: Material) {
    for (overlay, paint) in material.faces.into_iter().enumerate() {
        let Some(paint) = paint else { continue };
        // The same fluid flags the section mesher sets, so the material stage's water
        // reaches the distant tiles too and the LOD boundary keeps one look.
        let color = if material.fluid != 0 {
            fluid_face(paint.color, direction as usize)
        } else {
            paint.color
        };
        faces
            .entry(FaceKey {
                direction,
                plane: plane.to_bits(),
                overlay: overlay as u8,
                layer: paint.layer,
                light: paint.light,
                color,
                translucent: paint.translucent,
            })
            .or_default()
            .push(rect);
    }
}

#[cfg(test)]
fn mesh_columns(
    key: TileKey,
    columns: &[Column],
    covered: &FxHashSet<(i32, i32)>,
    covered_bounds: Option<[i32; 4]>,
    material: impl FnMut(&Span, Direction, [i32; 2]) -> Material,
) -> SectionMesh {
    mesh_columns_with_neighbors(key, columns, covered, covered_bounds, &[], material)
}
#[cfg(test)]
fn mesh_columns_with_neighbors(
    key: TileKey,
    columns: &[Column],
    covered: &FxHashSet<(i32, i32)>,
    covered_bounds: Option<[i32; 4]>,
    neighbors: &[Column],
    material: impl FnMut(&Span, Direction, [i32; 2]) -> Material,
) -> SectionMesh {
    assert!(neighbors.is_empty() || neighbors.len() == 4 * CELLS);
    let stride = key.size() / CELLS as i32;
    let columns: Vec<&Column> = columns.iter().collect();
    let edges = std::array::from_fn(|side| {
        neighbors
            .get(side * CELLS..(side + 1) * CELLS)
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(row, column)| EdgeColumn {
                start: row as i32 * stride,
                end: (row as i32 + 1) * stride,
                column,
            })
            .collect()
    });
    mesh_columns_with_edges(
        key,
        &columns,
        covered,
        covered_bounds,
        &edges,
        &[],
        material,
    )
}
/// From this tile level up a cell is 64 blocks wide or more: the strata
/// inside the ground cannot be told apart, so a column is its surface.
const HEIGHTMAP_LEVEL: u8 = 6;
/// `column` as a heightmap: its top solid span reaching down to the bottom of
/// the world (so the walls toward lower neighbours are one face of the
/// surface's material, merged along the cliff), and the fluid over it.
fn heightmap(column: &Column, mut fluid: impl FnMut(&Span) -> bool) -> Column {
    let Some((top, below)) = column.spans.split_last() else {
        return column.clone();
    };
    let ground = |span: &Span| Span {
        min: crate::chunk::MIN_Y as i16,
        ..*span
    };
    let mut spans = Spans::with_capacity(2);
    match below.last() {
        Some(under) if fluid(top) => {
            spans.push(ground(under));
            spans.push(*top);
        }
        _ if fluid(top) => spans.push(*top),
        _ => spans.push(ground(top)),
    }
    Column {
        surface: column.surface,
        spans,
    }
}
fn mesh_columns_with_edges(
    key: TileKey,
    columns: &[&Column],
    covered: &FxHashSet<(i32, i32)>,
    covered_bounds: Option<[i32; 4]>,
    neighbors: &[Vec<EdgeColumn<'_>>; 4],
    finer: &[TileKey],
    mut material: impl FnMut(&Span, Direction, [i32; 2]) -> Material,
) -> SectionMesh {
    assert_eq!(columns.len(), CELLS * CELLS);
    // The heightmap columns are built here and borrowed like the tile's own.
    let heightmap_columns: Vec<Column>;
    let heightmap_refs: Vec<&Column>;
    let heightmap_edges: [Vec<Column>; 4];
    let heightmap_neighbors: [Vec<EdgeColumn<'_>>; 4];
    let (columns, neighbors) = if key.level >= HEIGHTMAP_LEVEL {
        let [ox, _, oz] = key.origin();
        let mut fluid = |span: &Span| material(span, Direction::Up, [ox, oz]).fluid != 0;
        heightmap_columns = columns
            .iter()
            .map(|column| heightmap(column, &mut fluid))
            .collect();
        heightmap_edges = neighbors.each_ref().map(|side| {
            side.iter()
                .map(|edge| heightmap(edge.column, &mut fluid))
                .collect()
        });
        heightmap_refs = heightmap_columns.iter().collect();
        heightmap_neighbors = std::array::from_fn(|side| {
            neighbors[side]
                .iter()
                .zip(&heightmap_edges[side])
                .map(|(edge, column)| EdgeColumn {
                    start: edge.start,
                    end: edge.end,
                    column,
                })
                .collect()
        });
        (&heightmap_refs[..], &heightmap_neighbors)
    } else {
        (columns, neighbors)
    };
    let mut count = CELLS * CELLS;
    let edge_offsets = std::array::from_fn::<_, 4, _>(|side| {
        let start = count;
        count += neighbors[side].len();
        start
    });
    let stride = key.size() / CELLS as i32;
    let [ox, _, oz] = key.origin();
    // A cell meets full-detail or finer terrain: it is meshed in smaller steps.
    let cell_step = |x: usize, z: usize| {
        let (wx, wz) = (ox + x as i32 * stride, oz + z as i32 * stride);
        let overlaps = covered_bounds.is_some_and(|[x0, z0, x1, z1]| {
            wx < x1 && wx + stride > x0 && wz < z1 && wz + stride > z0
        });
        let mut step = if overlaps { stride.min(16) } else { stride };
        for other in finer {
            let [x, _, z] = other.origin();
            if wx < x + other.size() && x < wx + stride && wz < z + other.size() && z < wz + stride
            {
                step = step.min(other.size());
            }
        }
        // The cells of the coarsest tiles are thousands of blocks wide: a
        // few steps each keep the work bounded, the cut around finer terrain
        // coarse for as long as such a tile stands in for its children.
        step.max(stride / MAX_STEPS)
    };
    let filled = fill_buried_gaps(
        columns,
        neighbors,
        stride,
        |x, z| cell_step(x, z) == stride,
        |span| material(span, Direction::Up, [ox, oz]).fluid != 0,
    );
    // A cell's faces take the colours at its centre. The tile's neighbours'
    // columns only hide faces, so their colours never show.
    let centre = |i: usize| {
        if i < CELLS * CELLS {
            [
                ox + (i % CELLS) as i32 * stride + stride / 2,
                oz + (i / CELLS) as i32 * stride + stride / 2,
            ]
        } else {
            [ox, oz]
        }
    };
    let mut materials = Vec::new();
    let prepared: Vec<Vec<Segment<'_>>> = filled
        .iter()
        .map(|column| column.as_ref())
        .chain(neighbors.iter().flatten().map(|edge| edge.column))
        .enumerate()
        .map(|(index, column)| {
            let at = centre(index);
            materials.clear();
            materials.extend(column.spans.iter().map(|s| material(s, Direction::Up, at)));
            column
                .spans
                .iter()
                .enumerate()
                .filter(|(_, span)| span.min < span.max)
                .map(|(i, span)| {
                    let current = materials[i];
                    let top = !column.spans.get(i + 1).is_some_and(|next| {
                        next.min == span.max && hides(next, materials[i + 1], span, current)
                    });
                    let bottom = i == 0
                        || !(column.spans[i - 1].max == span.min
                            && hides(&column.spans[i - 1], materials[i - 1], span, current));
                    Segment {
                        span,
                        min: f32::from(span.min),
                        max: f32::from(span.max)
                            - if current.fluid != 0 && top {
                                1.0 / 9.0
                            } else {
                                0.0
                            },
                        material: current,
                        top,
                        bottom,
                    }
                })
                .collect()
        })
        .collect();
    let mut faces = Faces::new();
    let finer_keys: FxHashSet<_> = finer.iter().copied().collect();
    let mut finer_levels: Vec<_> = finer.iter().map(|key| key.level).collect();
    finer_levels.sort_unstable();
    finer_levels.dedup();
    let is_covered = |x: i32, z: i32| {
        covered.contains(&(x.div_euclid(16), z.div_euclid(16)))
            || finer_levels.iter().any(|&level| {
                let size = 16 << level;
                finer_keys.contains(&TileKey {
                    x: x.div_euclid(size),
                    z: z.div_euclid(size),
                    level,
                })
            })
    };
    for z in 0..CELLS {
        for x in 0..CELLS {
            let (wx, wz) = (ox + x as i32 * stride, oz + z as i32 * stride);
            let step = cell_step(x, z);
            let column = &prepared[z * CELLS + x];
            let own_sky = column.last().map_or(0, |top| top.span.light & 0xff);
            for dz in (0..stride).step_by(step as usize) {
                for dx in (0..stride).step_by(step as usize) {
                    if is_covered(wx + dx, wz + dz) {
                        continue;
                    }
                    let (x0, z0) = (
                        (x as i32 * stride + dx) as f32,
                        (z as i32 * stride + dz) as f32,
                    );
                    let (x1, z1) = (x0 + step as f32, z0 + step as f32);
                    for segment in column {
                        if segment.top {
                            add_face(
                                &mut faces,
                                Direction::Up,
                                segment.max,
                                Rect {
                                    u0: x0,
                                    v0: z0,
                                    u1: x1,
                                    v1: z1,
                                },
                                segment.material,
                            );
                        }
                        if segment.bottom {
                            add_face(
                                &mut faces,
                                Direction::Down,
                                segment.min,
                                Rect {
                                    u0: x0,
                                    v0: z0,
                                    u1: x1,
                                    v1: z1,
                                },
                                material(segment.span, Direction::Down, centre(z * CELLS + x)),
                            );
                        }
                        for (direction, nx, nz, edge, nw, plane, u0, u1) in [
                            (
                                Direction::West,
                                x as i32 - 1,
                                z as i32,
                                dx == 0,
                                [wx + dx - 1, wz + dz],
                                x0,
                                z0,
                                z1,
                            ),
                            (
                                Direction::East,
                                x as i32 + 1,
                                z as i32,
                                dx + step == stride,
                                [wx + dx + step, wz + dz],
                                x1,
                                z0,
                                z1,
                            ),
                            (
                                Direction::North,
                                x as i32,
                                z as i32 - 1,
                                dz == 0,
                                [wx + dx, wz + dz - 1],
                                z0,
                                x0,
                                x1,
                            ),
                            (
                                Direction::South,
                                x as i32,
                                z as i32 + 1,
                                dz + step == stride,
                                [wx + dx, wz + dz + step],
                                z1,
                                x0,
                                x1,
                            ),
                        ] {
                            // Near meshes may have culled against the real neighbor.
                            // Keep a finite seal where their full-detail area begins:
                            // opaque only, as the near water carries on into the far
                            // water and a seal shows through it as a wall.
                            if is_covered(nw[0], nw[1]) {
                                if segment.material.opaque {
                                    add_side(
                                        &mut faces,
                                        direction,
                                        plane,
                                        u0,
                                        u1,
                                        segment,
                                        &[],
                                        || material(segment.span, direction, centre(z * CELLS + x)),
                                        own_sky,
                                    );
                                }
                            } else if !edge {
                                add_side(
                                    &mut faces,
                                    direction,
                                    plane,
                                    u0,
                                    u1,
                                    segment,
                                    column,
                                    || material(segment.span, direction, centre(z * CELLS + x)),
                                    own_sky,
                                );
                            } else if (0..CELLS as i32).contains(&nx)
                                && (0..CELLS as i32).contains(&nz)
                            {
                                add_side(
                                    &mut faces,
                                    direction,
                                    plane,
                                    u0,
                                    u1,
                                    segment,
                                    &prepared[nz as usize * CELLS + nx as usize],
                                    || material(segment.span, direction, centre(z * CELLS + x)),
                                    own_sky,
                                );
                            } else {
                                let edge_side = match direction {
                                    Direction::West => 0,
                                    Direction::East => 1,
                                    Direction::North => 2,
                                    Direction::South => 3,
                                    _ => unreachable!(),
                                };
                                if neighbors[edge_side].is_empty() {
                                    add_side(
                                        &mut faces,
                                        direction,
                                        plane,
                                        u0,
                                        u1,
                                        segment,
                                        &[],
                                        || material(segment.span, direction, centre(z * CELLS + x)),
                                        own_sky,
                                    );
                                }
                                // A coarse face can meet several finer columns.
                                // Clip each interval independently to avoid missing walls.
                                for (i, edge) in neighbors[edge_side].iter().enumerate() {
                                    let start = u0.max(edge.start as f32);
                                    let end = u1.min(edge.end as f32);
                                    if start < end {
                                        add_side(
                                            &mut faces,
                                            direction,
                                            plane,
                                            start,
                                            end,
                                            segment,
                                            &prepared[edge_offsets[edge_side] + i],
                                            || {
                                                material(
                                                    segment.span,
                                                    direction,
                                                    centre(z * CELLS + x),
                                                )
                                            },
                                            own_sky,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut mesh = SectionMesh::default();
    for (key, mut rects) in faces {
        merge(&mut rects);
        for rect in rects {
            emit(&mut mesh, key, rect)
        }
    }
    mesh.finish_sprite_layers();
    mesh
}

#[cfg(test)]
mod refinement_tests {
    use super::*;
    #[test]
    fn fallback_mesh_leaves_exactly_the_fine_tiles_footprints_empty() {
        let parent = TileKey {
            x: -1,
            z: -1,
            level: 5,
        };
        // Smaller than one coarse cell, away from the parent's outer edges.
        let fine = TileKey {
            x: -17,
            z: -17,
            level: 0,
        };
        let columns = vec![
            Column {
                surface: None,
                spans: vec![Span {
                    min: 0,
                    max: 8,
                    state: BlockStateId(1),
                    biome: 0,
                    light: 240,
                }]
                .into()
            };
            256
        ];
        let columns: Vec<&Column> = columns.iter().collect();
        let mesh = mesh_columns_with_edges(
            parent,
            &columns,
            &FxHashSet::default(),
            None,
            &[Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            &[fine],
            |_, _, _| Material {
                faces: [
                    Some(Paint {
                        layer: 1,
                        light: 240,
                        color: u32::MAX,
                        translucent: false,
                    }),
                    None,
                ],
                fluid: 0,
                opaque: true,
            },
        );
        let [ox, _, oz] = parent.origin();
        let [fx, _, fz] = fine.origin();
        let mut area = 0.0;
        for quad in mesh.solid.as_chunks::<4>().0 {
            if !quad.iter().all(|v| v.pos[1] == 8.0) {
                continue;
            }
            let a = glam::Vec3::from(quad[0].pos);
            let b = glam::Vec3::from(quad[1].pos);
            let c = glam::Vec3::from(quad[2].pos);
            let normal = (b - a).cross(c - a);
            area += normal.y;
            let min_x = quad.iter().map(|v| v.pos[0]).fold(f32::INFINITY, f32::min) + ox as f32;
            let max_x = quad
                .iter()
                .map(|v| v.pos[0])
                .fold(f32::NEG_INFINITY, f32::max)
                + ox as f32;
            let min_z = quad.iter().map(|v| v.pos[2]).fold(f32::INFINITY, f32::min) + oz as f32;
            let max_z = quad
                .iter()
                .map(|v| v.pos[2])
                .fold(f32::NEG_INFINITY, f32::max)
                + oz as f32;
            assert!(
                !(min_x < (fx + 16) as f32
                    && max_x > fx as f32
                    && min_z < (fz + 16) as f32
                    && max_z > fz as f32)
            );
        }
        assert_eq!(area, (parent.size() * parent.size() - 256) as f32);
    }
}

/// The tile's columns with the gaps no open air reaches filled with the
/// material below them, or the fluid above them (kelp and seagrass leave gaps
/// in water): a cave seen from nowhere adds no faces. A gap is open when a
/// neighbouring column's top lies below the gap's top, so that column's open
/// air looks into it, when a gap of the tile's neighbour meets it, or when it
/// meets an open gap of a neighbouring column: the air under a canopy reaches
/// the sky through any clearing, where filling it raised pillars of the floor
/// block. Unknown neighbours, and cells that `whole` rejects (they meet
/// full-detail or finer terrain), count as open; those cells keep their gaps.
fn fill_buried_gaps<'a>(
    columns: &'a [&'a Column],
    neighbors: &'a [Vec<EdgeColumn<'a>>; 4],
    stride: i32,
    whole: impl Fn(usize, usize) -> bool,
    mut fluid: impl FnMut(&Span) -> bool,
) -> Vec<std::borrow::Cow<'a, Column>> {
    use std::borrow::Cow;
    let top = |column: &Column| {
        column
            .spans
            .iter()
            .rev()
            .find(|span| span.min < span.max)
            .map_or(i16::MIN, |span| span.max)
    };
    // Each column's gaps: the index of the span above, the gap's bottom and top.
    let gaps_of = |column: &Column| -> Vec<(usize, i16, i16)> {
        column
            .spans
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0].max < pair[1].min)
            .map(|(k, pair)| (k + 1, pair[0].max, pair[1].min))
            .collect()
    };
    let meets =
        |(lo, hi): (i16, i16), (other_lo, other_hi): (i16, i16)| lo < other_hi && other_lo < hi;
    let tops: [i16; CELLS * CELLS] = std::array::from_fn(|i| {
        if whole(i % CELLS, i / CELLS) {
            top(columns[i])
        } else {
            i16::MIN
        }
    });
    let gaps: Vec<Vec<(usize, i16, i16)>> = (0..CELLS * CELLS)
        .map(|i| {
            if whole(i % CELLS, i / CELLS) {
                gaps_of(columns[i])
            } else {
                Vec::new()
            }
        })
        .collect();
    let touching = |side: usize, cell: usize| {
        let (start, end) = (cell as i32 * stride, (cell as i32 + 1) * stride);
        neighbors[side]
            .iter()
            .filter(move |edge| edge.start < end && start < edge.end)
    };
    let edge = |side: usize, cell: usize| {
        touching(side, cell)
            .map(|edge| top(edge.column))
            .min()
            .unwrap_or(i16::MIN)
    };
    // A gap of the tile's neighbour may be open through tiles this one can't
    // see: it counts as open.
    let edge_gap = |side: usize, cell: usize, gap: (i16, i16)| {
        touching(side, cell).any(|edge| {
            gaps_of(edge.column)
                .iter()
                .any(|&(_, lo, hi)| meets((lo, hi), gap))
        })
    };
    let around = |i: usize| {
        let (x, z) = (i % CELLS, i / CELLS);
        [
            (x > 0).then(|| i - 1),
            (x + 1 < CELLS).then(|| i + 1),
            (z > 0).then(|| i - CELLS),
            (z + 1 < CELLS).then(|| i + CELLS),
        ]
        .into_iter()
        .flatten()
    };
    let mut open: Vec<Vec<bool>> = gaps.iter().map(|g| vec![false; g.len()]).collect();
    let mut reached = Vec::new();
    for i in 0..CELLS * CELLS {
        let (x, z) = (i % CELLS, i / CELLS);
        let lowest = [
            if x > 0 { tops[i - 1] } else { edge(0, z) },
            if x + 1 < CELLS {
                tops[i + 1]
            } else {
                edge(1, z)
            },
            if z > 0 { tops[i - CELLS] } else { edge(2, x) },
            if z + 1 < CELLS {
                tops[i + CELLS]
            } else {
                edge(3, x)
            },
        ]
        .into_iter()
        .min()
        .unwrap_or(i16::MIN);
        for (g, &(_, lo, hi)) in gaps[i].iter().enumerate() {
            let gap = (lo, hi);
            if hi > lowest
                || (x == 0 && edge_gap(0, z, gap))
                || (x + 1 == CELLS && edge_gap(1, z, gap))
                || (z == 0 && edge_gap(2, x, gap))
                || (z + 1 == CELLS && edge_gap(3, x, gap))
            {
                open[i][g] = true;
                reached.push((i, g));
            }
        }
    }
    while let Some((i, g)) = reached.pop() {
        let (_, lo, hi) = gaps[i][g];
        for j in around(i) {
            for (h, &(_, other_lo, other_hi)) in gaps[j].iter().enumerate() {
                if !open[j][h] && meets((lo, hi), (other_lo, other_hi)) {
                    open[j][h] = true;
                    reached.push((j, h));
                }
            }
        }
    }
    columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let column = *column;
            if open[i].iter().all(|&open| open) {
                return Cow::Borrowed(column);
            }
            let mut filled = Column {
                surface: column.surface,
                spans: Spans::with_capacity(column.spans.len()),
            };
            for (k, &span) in column.spans.iter().enumerate() {
                let mut span = span;
                let buried = gaps[i]
                    .iter()
                    .zip(&open[i])
                    .any(|(&(above, _, _), &open)| above == k && !open);
                if buried && let Some(below) = filled.spans.last_mut() {
                    if fluid(&span) {
                        span.min = below.max;
                    } else {
                        below.max = span.min;
                    }
                }
                filled.push(span);
            }
            Cow::Owned(filled)
        })
        .collect()
}

/// Adds the parts of `segment`'s side not hidden by `neighbor`, the column
/// it faces. Parts above the neighbour's top face open air: reduced columns
/// and the seals at full-detail edges expose sides that were buried (their
/// recorded light is a cave's or none), so those take at least the sky light
/// over the top of the neighbour (or of this column, with no neighbour).
fn add_side(
    faces: &mut Faces,
    direction: Direction,
    plane: f32,
    u0: f32,
    u1: f32,
    segment: &Segment<'_>,
    neighbor: &[Segment<'_>],
    mut side: impl FnMut() -> Material,
    own_sky: u16,
) {
    let (open, sky) = neighbor.last().map_or((f32::NEG_INFINITY, own_sky), |top| {
        (f32::from(top.span.max), top.span.light & 0xff)
    });
    let mut cached = None;
    let mut add = |v0: f32, v1: f32| {
        let side = match cached {
            Some(side) => side,
            None => {
                let side = side();
                cached = Some(side);
                side
            }
        };
        let mut lit = side;
        for paint in lit.faces.iter_mut().flatten() {
            paint.light = (paint.light & 0xff00) | (paint.light & 0xff).max(sky);
        }
        let split = open.clamp(v0, v1);
        for (v0, v1, material) in [(v0, split, side), (split, v1, lit)] {
            if v0 < v1 {
                add_face(faces, direction, plane, Rect { u0, v0, u1, v1 }, material);
            }
        }
    };
    let mut floor = segment.min;
    for next in neighbor {
        if next.max <= floor
            || next.min >= segment.max
            || !hides(next.span, next.material, segment.span, segment.material)
        {
            continue;
        }
        if floor < next.min {
            add(floor, next.min.min(segment.max));
        }
        floor = floor.max(next.max);
        if floor >= segment.max {
            break;
        }
    }
    if floor < segment.max {
        add(floor, segment.max);
    }
}

fn merge(rects: &mut Vec<Rect>) {
    rects.sort_unstable_by(|a, b| {
        a.v0.total_cmp(&b.v0)
            .then(a.v1.total_cmp(&b.v1))
            .then(a.u0.total_cmp(&b.u0))
            .then(a.u1.total_cmp(&b.u1))
    });
    let mut end = 0;
    for i in 0..rects.len() {
        let rect = rects[i];
        if end > 0
            && rects[end - 1].v0 == rect.v0
            && rects[end - 1].v1 == rect.v1
            && rects[end - 1].u1 == rect.u0
        {
            rects[end - 1].u1 = rect.u1;
        } else {
            rects[end] = rect;
            end += 1
        }
    }
    rects.truncate(end);
    rects.sort_unstable_by(|a, b| {
        a.u0.total_cmp(&b.u0)
            .then(a.u1.total_cmp(&b.u1))
            .then(a.v0.total_cmp(&b.v0))
            .then(a.v1.total_cmp(&b.v1))
    });
    end = 0;
    for i in 0..rects.len() {
        let rect = rects[i];
        if end > 0
            && rects[end - 1].u0 == rect.u0
            && rects[end - 1].u1 == rect.u1
            && rects[end - 1].v1 == rect.v0
        {
            rects[end - 1].v1 = rect.v1;
        } else {
            rects[end] = rect;
            end += 1
        }
    }
    rects.truncate(end);
}
/// The widest face `emit` writes, in blocks: its UVs, counted from the face's
/// corner, must stay within `TerrainVertex::tiled`'s range.
const MAX_FACE: f32 = 1024.0;
/// A face that would be more than this many `MAX_FACE` pieces is one quad
/// whose UVs are scaled to fit: a cell of the coarsest tiles is thousands to
/// millions of blocks wide, and the texture repeats within it are far below a
/// pixel.
const MAX_PIECES: f32 = 4.0;
/// Steps along a cell of a tile cut around finer terrain, at most.
const MAX_STEPS: i32 = 32;
fn emit(mesh: &mut SectionMesh, key: FaceKey, rect: Rect) {
    let pieces = ((rect.u1 - rect.u0) / MAX_FACE).ceil() * ((rect.v1 - rect.v0) / MAX_FACE).ceil();
    if pieces > MAX_PIECES {
        let extent = (rect.u1 - rect.u0).max(rect.v1 - rect.v0);
        emit_quad(mesh, key, rect, (MAX_FACE / extent).min(1.0));
        return;
    }
    let mut u0 = rect.u0;
    while u0 < rect.u1 {
        let u1 = rect.u1.min(u0 + MAX_FACE);
        let mut v0 = rect.v0;
        while v0 < rect.v1 {
            let v1 = rect.v1.min(v0 + MAX_FACE);
            emit_quad(mesh, key, Rect { u0, v0, u1, v1 }, 1.0);
            v0 = v1;
        }
        u0 = u1;
    }
}
/// `uv_scale` shrinks the UVs, in blocks from the face's corner, of a face
/// too wide for them.
fn emit_quad(mesh: &mut SectionMesh, key: FaceKey, rect: Rect, uv_scale: f32) {
    mesh.note_sprite_layer(key.layer);
    let Rect { u0, v0, u1, v1 } = rect;
    let p = f32::from_bits(key.plane);
    let mut points = match key.direction {
        Direction::Up | Direction::Down => [[u0, p, v1], [u1, p, v1], [u1, p, v0], [u0, p, v0]],
        Direction::North | Direction::South => [[u0, v1, p], [u1, v1, p], [u1, v0, p], [u0, v0, p]],
        Direction::West | Direction::East => [[p, v1, u1], [p, v1, u0], [p, v0, u0], [p, v0, u1]],
    };
    if matches!(
        key.direction,
        Direction::Down | Direction::South | Direction::East
    ) {
        points.reverse()
    }
    let out = if key.translucent {
        &mut mesh.translucent
    } else {
        &mut mesh.solid
    };
    // Tile origins are integer multiples of a block, so local coordinates
    // retain the world texture phase without distant-coordinate precision loss;
    // a whole number of blocks off each UV keeps it (terrain.wgsl's `tile_uv`
    // repeats the sprite per block) and counts it from the face's corner.
    let uvs = points.map(|pos| match key.direction {
        Direction::Up | Direction::Down => [pos[0], -pos[2]],
        Direction::North | Direction::South => [pos[0], -pos[1]],
        Direction::West | Direction::East => [pos[2], -pos[1]],
    });
    let corner = [0, 1].map(|i| {
        uvs.iter()
            .map(|uv| uv[i])
            .fold(f32::INFINITY, f32::min)
            .floor()
    });
    out.extend(points.into_iter().zip(uvs).map(|(pos, uv)| {
        TerrainVertex::tiled(
            pos,
            [
                (uv[0] - corner[0]) * uv_scale,
                (uv[1] - corner[1]) * uv_scale,
            ],
            key.layer,
            key.color,
            key.light,
        )
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn span(min: i16, max: i16, state: u32) -> Span {
        Span {
            min,
            max,
            state: BlockStateId(state),
            biome: 0,
            light: 240,
        }
    }
    fn columns() -> Vec<Column> {
        (0..CELLS * CELLS)
            .map(|_| Column {
                surface: None,
                spans: Spans::new(),
            })
            .collect()
    }
    fn material(span: &Span, _: Direction, _: [i32; 2]) -> Material {
        let water = span.state.0 == 2;
        Material {
            faces: [
                Some(Paint {
                    layer: span.state.0 as u16,
                    light: span.light,
                    color: u32::MAX,
                    translucent: water,
                }),
                None,
            ],
            fluid: u8::from(water),
            opaque: !water,
        }
    }
    fn mesh(columns: &[Column]) -> SectionMesh {
        mesh_columns(
            TileKey {
                x: 0,
                z: 0,
                level: 0,
            },
            columns,
            &FxHashSet::default(),
            None,
            material,
        )
    }
    fn tiled_uv(v: &TerrainVertex) -> [f32; 2] {
        v.uv.map(|u| f32::from(u) / crate::mesher::TILED_UV_STEPS)
    }
    #[test]
    fn wide_faces_split_and_keep_their_texture_phase() {
        let mut mesh = SectionMesh::default();
        let key = FaceKey {
            direction: Direction::Up,
            plane: 64.0f32.to_bits(),
            overlay: 0,
            layer: 1,
            light: 0,
            color: 0,
            translucent: false,
        };
        emit(
            &mut mesh,
            key,
            Rect {
                u0: 0.0,
                v0: 0.5,
                u1: 2500.0,
                v1: 16.0,
            },
        );
        // 2500 blocks: 1024 + 1024 + 452.
        assert_eq!(mesh.solid.len(), 3 * 4);
        let mut area = 0.0;
        for q in mesh.solid.as_chunks::<4>().0 {
            let x =
                |f: fn(f32, f32) -> f32, i: usize| q.iter().map(|v| v.pos[i]).reduce(f).unwrap();
            area += (x(f32::max, 0) - x(f32::min, 0)) * (x(f32::max, 2) - x(f32::min, 2));
            for v in q {
                // UVs follow the world position modulo a block.
                let uv = tiled_uv(v);
                assert_eq!((uv[0] - v.pos[0]).rem_euclid(1.0), 0.0);
                assert_eq!((uv[1] + v.pos[2]).rem_euclid(1.0), 0.0);
                assert!(uv[0] <= MAX_FACE + 1.0 && uv[1] <= MAX_FACE + 1.0);
            }
        }
        assert_eq!(area, 2500.0 * 15.5);
    }
    #[test]
    fn far_columns_are_heightmaps_of_their_surface_and_water() {
        let fluid = |s: &Span| s.state.0 == 2;
        let strata = Column {
            surface: None,
            spans: vec![span(0, 60, 1), span(60, 63, 3), span(63, 64, 4)].into(),
        };
        let flat = heightmap(&strata, fluid);
        assert_eq!(flat.spans, [span(-64, 64, 4)]);
        let sea = Column {
            surface: None,
            spans: vec![span(0, 50, 1), span(50, 62, 2)].into(),
        };
        let flat = heightmap(&sea, fluid);
        assert_eq!(flat.spans, [span(-64, 50, 1), span(50, 62, 2)]);
        assert!(heightmap(&columns()[0], fluid).spans.is_empty());
        // A rolling landscape of strata: far tiles need a fraction of the quads.
        let rolling: Vec<Column> = (0..CELLS * CELLS)
            .map(|i| {
                let top = 64 + (i * 7 % 5) as i16;
                Column {
                    surface: None,
                    spans: vec![
                        span(0, top - 4, 1),
                        span(top - 4, top - 1, 3),
                        span(top - 1, top, 4),
                    ]
                    .into(),
                }
            })
            .collect();
        let quads = |level| {
            let key = TileKey { x: 0, z: 0, level };
            let mesh = mesh_columns(key, &rolling, &FxHashSet::default(), None, material);
            mesh.solid.len() / 4
        };
        let (voxel, height) = (quads(HEIGHTMAP_LEVEL - 1), quads(HEIGHTMAP_LEVEL));
        assert!(height * 3 < voxel * 2, "{height} quads against {voxel}");
    }
    #[test]
    fn world_sized_faces_are_one_quad_with_uvs_that_fit() {
        let mut mesh = SectionMesh::default();
        let key = FaceKey {
            direction: Direction::Up,
            plane: 64.0f32.to_bits(),
            overlay: 0,
            layer: 1,
            light: 0,
            color: 0,
            translucent: false,
        };
        let size = 4_194_304.0;
        emit(
            &mut mesh,
            key,
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: size,
                v1: size / 2.0,
            },
        );
        assert_eq!(mesh.solid.len(), 4);
        let corners: Vec<[f32; 3]> = mesh.solid.iter().map(|v| v.pos).collect();
        assert!(
            corners.iter().any(|p| p[0] == size && p[2] == size / 2.0),
            "{corners:?}"
        );
        let top = mesh
            .solid
            .iter()
            .map(tiled_uv)
            .fold(0.0f32, |m, uv| m.max(uv[0]).max(uv[1]));
        assert!(top <= MAX_FACE + 1.0, "UV reaches {top}");
        assert!(
            top > MAX_FACE / 4.0,
            "the texture still repeats across the face"
        );
    }
    fn normal(q: &[TerrainVertex; 4]) -> glam::Vec3 {
        let p = |i: usize| glam::Vec3::from(q[i].pos);
        (p(1) - p(0)).cross(p(2) - p(0))
    }
    #[test]
    fn flat_solid_cube_merges_to_six_faces_with_block_scaled_uvs() {
        let mut columns = columns();
        for column in &mut columns {
            column.spans = vec![span(0, 16, 1)].into()
        }
        let mesh = mesh(&columns);
        assert_eq!(mesh.solid.len(), 6 * 4);
        assert!(mesh.translucent.is_empty());
        for q in mesh.solid.as_chunks::<4>().0.iter() {
            let center = q
                .iter()
                .map(|v| glam::Vec3::from(v.pos))
                .sum::<glam::Vec3>()
                / 4.0;
            assert!(normal(q).dot(center - glam::Vec3::splat(8.0)) > 0.0);
            assert_eq!(
                q.iter()
                    .map(|v| tiled_uv(v)[0])
                    .fold(f32::NEG_INFINITY, f32::max)
                    - q.iter()
                        .map(|v| tiled_uv(v)[0])
                        .fold(f32::INFINITY, f32::min),
                16.0
            );
            assert_eq!(
                q.iter()
                    .map(|v| tiled_uv(v)[1])
                    .fold(f32::NEG_INFINITY, f32::max)
                    - q.iter()
                        .map(|v| tiled_uv(v)[1])
                        .fold(f32::INFINITY, f32::min),
                16.0
            );
        }
    }
    #[test]
    fn gaps_and_real_bottoms_remain_open_without_floor_curtains() {
        let mut columns = columns();
        columns[0].spans = vec![span(3, 5, 1), span(8, 9, 1)].into();
        let mesh = mesh(&columns);
        assert_eq!(mesh.solid.len(), 12 * 4);
        assert!(
            mesh.solid
                .iter()
                .all(|v| (3.0..=5.0).contains(&v.pos[1]) || (8.0..=9.0).contains(&v.pos[1]))
        );
        assert!(
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .any(|q| q.iter().all(|v| v.pos[1] == 8.0) && normal(q).y < 0.0)
        );
    }
    #[test]
    fn neighbor_hides_only_its_occupied_side_intervals() {
        let mut columns = columns();
        columns[0].spans = vec![span(0, 10, 1)].into();
        columns[1].spans = vec![span(0, 3, 1), span(7, 10, 1)].into();
        let mesh = mesh(&columns);
        let opening: Vec<_> = mesh
            .solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| q.iter().all(|v| v.pos[0] == 1.0) && normal(q).x > 0.0)
            .collect();
        assert_eq!(opening.len(), 1);
        assert_eq!(
            opening[0]
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::INFINITY, f32::min),
            3.0
        );
        assert_eq!(
            opening[0]
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::NEG_INFINITY, f32::max),
            7.0
        );
    }
    #[test]
    fn exposed_buried_sides_take_the_sky_light_over_their_neighbor() {
        let mut columns = columns();
        let dark = |min, max| Span {
            light: 0,
            ..span(min, max, 1)
        };
        // A reduced column whose lower blocks were buried (no light), next to
        // a lower column, and to one with a cave below its top.
        columns[0].spans = vec![dark(0, 9), span(9, 10, 1)].into();
        columns[1].spans = vec![span(0, 4, 1)].into();
        columns[CELLS].spans = vec![span(0, 2, 1), span(6, 12, 1)].into();
        let mesh = mesh(&columns);
        let quads = mesh.solid.as_chunks::<4>().0;
        let bounds = |q: &[TerrainVertex; 4]| {
            let ys = q.iter().map(|v| v.pos[1]);
            (
                ys.clone().fold(f32::INFINITY, f32::min),
                ys.fold(0.0, f32::max),
            )
        };
        // Over the lower neighbour: open air, lit like its top.
        let open: Vec<_> = quads
            .iter()
            .filter(|q| q.iter().all(|v| v.pos[0] == 1.0 && v.pos[2] <= 1.0) && normal(q).x > 0.0)
            .collect();
        assert_eq!(open.len(), 1);
        assert_eq!(bounds(open[0]), (4.0, 10.0));
        assert!(open[0].iter().all(|v| v.light == 240));
        // Into the cave: its recorded light.
        let cave: Vec<_> = quads
            .iter()
            .filter(|q| q.iter().all(|v| v.pos[2] == 1.0 && v.pos[0] <= 1.0) && normal(q).z > 0.0)
            .collect();
        assert_eq!(cave.len(), 1);
        assert_eq!(bounds(cave[0]), (2.0, 6.0));
        assert!(cave[0].iter().all(|v| v.light == 0));
    }
    #[test]
    fn air_under_a_canopy_stays_open_through_a_clearing() {
        let column = |spans: Vec<Span>| Column {
            surface: None,
            spans: spans.into(),
        };
        // The tile's neighbours are solid hills: no open air from the sides.
        let hill_column = column(vec![span(0, 20, 1)]);
        let hill = || EdgeColumn {
            start: 0,
            end: CELLS as i32,
            column: &hill_column,
        };
        let neighbors = [vec![hill()], vec![hill()], vec![hill()], vec![hill()]];
        let fill = |columns: &[Column]| -> Vec<Vec<Span>> {
            let columns: Vec<&Column> = columns.iter().collect();
            fill_buried_gaps(&columns, &neighbors, 1, |_, _| true, |_| false)
                .iter()
                .map(|filled| filled.spans.as_slice().to_vec())
                .collect()
        };
        let mut forest = vec![column(vec![span(0, 5, 1), span(10, 14, 3)]); CELLS * CELLS];
        // A closed canopy hides the floor: filling it under there shows nowhere.
        assert!(fill(&forest).iter().all(|spans| spans[0].max == 10));
        // One clearing lets the sky in: the air under every tree stays air,
        // not a pillar of the floor block up to the leaves.
        forest[5 * CELLS + 5] = column(vec![span(0, 5, 1)]);
        let filled = fill(&forest);
        for (filled, column) in filled.iter().zip(&forest) {
            assert_eq!(filled.as_slice(), column.spans.as_slice());
        }
        // A cave two columns wide sealed in rock is still filled.
        let mut rock = vec![column(vec![span(0, 20, 1)]); CELLS * CELLS];
        for cave in [5 * CELLS + 5, 5 * CELLS + 6] {
            rock[cave] = column(vec![span(0, 8, 1), span(12, 20, 1)]);
        }
        assert!(
            fill(&rock)
                .iter()
                .all(|spans| spans.as_slice() == [span(0, 20, 1)])
        );
    }
    #[test]
    fn caves_no_neighbor_sees_into_add_no_faces() {
        let mut columns = columns();
        for column in &mut columns {
            column.spans = vec![span(0, 20, 1)].into();
        }
        let cave = 5 * CELLS + 5;
        columns[cave].spans = vec![span(0, 8, 1), span(12, 20, 1)].into();
        let at = |mesh: &SectionMesh, y: f32| {
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|q| q.iter().all(|v| v.pos[1] == y))
                .count()
        };
        // Closed on every side: filled, no floor or ceiling.
        let closed = mesh(&columns);
        assert_eq!((at(&closed, 8.0), at(&closed, 12.0)), (0, 0));
        assert!(
            closed
                .solid
                .iter()
                .all(|v| v.pos[1] == 0.0 || v.pos[1] == 20.0)
        );
        // A lower neighbour's open air looks in: the cave stays.
        columns[cave + 1].spans = vec![span(0, 10, 1)].into();
        let open = mesh(&columns);
        assert_eq!((at(&open, 8.0), at(&open, 12.0)), (1, 1));
        // Kelp leaves a gap in water: water fills it, without inner faces.
        for column in &mut columns {
            column.spans = vec![span(0, 5, 1), span(5, 10, 2)].into();
        }
        columns[cave].spans = vec![span(0, 5, 1), span(5, 7, 2), span(8, 10, 2)].into();
        let water = mesh(&columns);
        assert!(
            water
                .translucent
                .iter()
                .all(|v| v.pos[1] != 7.0 && v.pos[1] != 8.0)
        );
        let uniform = mesh(&vec![columns[0].clone(); CELLS * CELLS]);
        assert_eq!(water.translucent.len(), uniform.translucent.len());
    }
    #[test]
    fn greedy_faces_preserve_material_and_light_boundaries() {
        let mut columns = columns();
        columns[0].spans = vec![span(0, 1, 1)].into();
        columns[1].spans = vec![span(0, 1, 3)].into();
        columns[2].spans = vec![Span {
            light: 16,
            ..span(0, 1, 3)
        }]
        .into();
        let mesh = mesh(&columns);
        let tops: Vec<_> = mesh
            .solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| normal(q).y > 0.0)
            .collect();
        assert_eq!(tops.len(), 3);
        assert_eq!(tops.iter().filter(|q| q[0].layer == 3).count(), 2);
        assert_eq!(tops.iter().filter(|q| q[0].light == 16).count(), 1);
        assert!(!mesh.solid.as_chunks::<4>().0.iter().any(|q| {
            normal(q).x != 0.0 && q.iter().all(|v| v.pos[0] == 1.0 || v.pos[0] == 2.0)
        }));
    }
    #[test]
    fn water_keeps_ground_surface_and_culls_shared_fluid_faces() {
        let mut columns = columns();
        columns[0].spans = vec![span(0, 3, 1), span(3, 6, 2)].into();
        columns[1].spans = columns[0].spans.clone();
        let mesh = mesh(&columns);
        assert!(
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .any(|q| q.iter().all(|v| v.pos[1] == 3.0) && normal(q).y > 0.0)
        );
        assert!(
            mesh.translucent
                .as_chunks::<4>()
                .0
                .iter()
                .all(|q| !(q.iter().all(|v| v.pos[0] == 1.0) && normal(q).x != 0.0))
        );
        assert!(mesh.translucent.as_chunks::<4>().0.iter().any(|q| {
            q.iter()
                .all(|v| (v.pos[1] - (6.0 - 1.0 / 9.0)).abs() < 0.001)
                && normal(q).y > 0.0
        }));
    }
    #[test]
    fn coarse_edge_clips_separate_fine_neighbor_heights() {
        let mut columns = columns();
        for column in &mut columns {
            column.spans = vec![span(0, 16, 1)].into();
        }
        let key = TileKey {
            x: -1,
            z: -1,
            level: 1,
        };
        let mut neighbors: [Vec<EdgeColumn<'_>>; 4] = std::array::from_fn(|_| {
            vec![EdgeColumn {
                start: 0,
                end: 32,
                column: &columns[0],
            }]
        });
        // One fine column is lower than both the coarse cell and its sibling.
        // Sampling only the first neighbor would incorrectly hide this wall.
        let low = Column {
            surface: None,
            spans: vec![span(0, 8, 1)].into(),
        };
        neighbors[0] = vec![
            EdgeColumn {
                start: 0,
                end: 1,
                column: &columns[0],
            },
            EdgeColumn {
                start: 1,
                end: 2,
                column: &low,
            },
            EdgeColumn {
                start: 2,
                end: 32,
                column: &columns[0],
            },
        ];
        let columns: Vec<&Column> = columns.iter().collect();
        let mesh = mesh_columns_with_edges(
            key,
            &columns,
            &FxHashSet::default(),
            None,
            &neighbors,
            &[],
            material,
        );
        let walls: Vec<_> = mesh
            .solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|quad| normal(quad).x < 0.0)
            .collect();
        assert_eq!(walls.len(), 1);
        let wall = walls[0];
        assert!(wall.iter().all(|v| v.pos[0] == 0.0
            && (1.0..=2.0).contains(&v.pos[2])
            && (8.0..=16.0).contains(&v.pos[1])));
        assert_eq!(normal(wall).length(), 8.0);
    }
    #[test]
    fn tile_halo_culls_shared_walls_but_keeps_steps_and_gaps() {
        let mut columns = columns();
        for column in &mut columns {
            column.spans = vec![span(0, 16, 1)].into();
        }
        let mut neighbors = columns[..4 * CELLS].to_vec();
        let key = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        let covered = FxHashSet::default();
        let mesh = mesh_columns_with_neighbors(key, &columns, &covered, None, &neighbors, material);
        assert_eq!(
            mesh.solid.len(),
            2 * 4,
            "only the top and real bottom remain"
        );
        assert!(
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .all(|q| normal(q).y != 0.0)
        );

        for column in &mut neighbors[CELLS..2 * CELLS] {
            column.spans = vec![span(0, 8, 1)].into();
        }
        for column in &mut neighbors[..CELLS] {
            column.spans = vec![span(0, 3, 1), span(7, 16, 1)].into();
        }
        let mesh = mesh_columns_with_neighbors(key, &columns, &covered, None, &neighbors, material);
        assert_eq!(mesh.solid.len(), 4 * 4);
        let sides: Vec<_> = mesh
            .solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| normal(q).y == 0.0)
            .collect();
        assert_eq!(sides.len(), 2);
        for q in sides {
            let min = q.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min);
            let max = q.iter().map(|v| v.pos[1]).fold(f32::NEG_INFINITY, f32::max);
            if normal(q).x > 0.0 {
                assert_eq!((min, max), (8.0, 16.0));
            } else {
                assert!(normal(q).x < 0.0);
                assert_eq!((min, max), (3.0, 7.0));
            }
        }
    }
    #[test]
    fn negative_parent_clips_chunks_without_internal_piece_walls() {
        let mut columns = columns();
        for column in &mut columns {
            column.spans = vec![span(10, 12, 1)].into()
        }
        let key = TileKey {
            x: -1,
            z: -1,
            level: 5,
        };
        let [ox, _, oz] = key.origin();
        let mut covered = FxHashSet::default();
        covered.insert((ox.div_euclid(16), oz.div_euclid(16)));
        let mesh = mesh_columns(
            key,
            &columns,
            &covered,
            Some([ox, oz, ox + 16, oz + 16]),
            material,
        );
        assert!(mesh.solid.as_chunks::<4>().0.iter().all(|q| {
            if normal(q).y == 0.0 {
                return true;
            }
            let center = q
                .iter()
                .map(|v| glam::Vec3::from(v.pos))
                .sum::<glam::Vec3>()
                / 4.0;
            !(center.x < 16.0 && center.z < 16.0)
        }));
        let cuts: Vec<_> = mesh
            .solid
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| {
                (q.iter().all(|v| v.pos[0] == 16.0) && normal(q).x < 0.0)
                    || (q.iter().all(|v| v.pos[2] == 16.0) && normal(q).z < 0.0)
            })
            .collect();
        assert_eq!(cuts.len(), 2);
        assert!(
            cuts.iter()
                .all(|q| q.iter().all(|v| (10.0..=12.0).contains(&v.pos[1]))),
            "cut faces keep finite voxel bounds"
        );
    }
}
