//! Leaf culling: decides which leaf faces a section may draw solid or drop.
//!
//! The mask is built on the mesh worker from the same padded block snapshot the
//! rest of the mesher reads, so a decision never needs a chunk lookup on the
//! render thread. It classifies each padded cell as a leaf, a full opaque cube,
//! open (air, fluids, partial models, anything else) or unknown (a missing
//! chunk, a Y outside the column, or the mask's outer shell). Only leaves can
//! hide or solidify anything; unknown and open cells always expose a face.
//!
//! `Solid` performs an immediate 6-neighbour occlusion check, solidifying fully
//! enclosed leaves into the opaque layer without cutout testing. `Solid
//! Aggressive` runs a multi-source breadth-first search from leaves touching
//! openings, solidifying leaves with a single open face to thin the canopy shell
//! by one layer. `Hollow` directly drops faces shared between two leaves within
//! the section, preserving boundaries and cavity faces.

use super::{B, Info, P, Padded, pi};
use crate::{blocks::RenderShape, chunk::Chunk, graphics::LeafCullingMode};

/// A cell of the padded snapshot, as far as leaf culling cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cell {
    /// Air, a fluid, a partial model, glass or any other non-occluding block.
    Open,
    /// `LeavesBlock`.
    Leaf,
    /// A full opaque cube (see [`opaque_cube`]).
    Opaque,
    /// Missing chunk, missing section or the mask's outer shell.
    Unknown,
}

/// Whether a state is a full opaque cube: `RenderShape::Model`, `can_occlude`,
/// `solid_render` and a full collision shape. Glass, fluids, slabs, stairs,
/// chests and models that only partly fill their block are not.
pub(super) fn opaque_cube(
    shape: RenderShape,
    can_occlude: bool,
    solid_render: bool,
    full_collision: bool,
) -> bool {
    shape == RenderShape::Model && can_occlude && solid_render && full_collision
}

const UNVISITED: u8 = u8::MAX;

const STRIDE_X: isize = 1;
const STRIDE_Z: isize = P as isize;
const STRIDE_Y: isize = (P * P) as isize;

const ADJACENT_STRIDES: [isize; 6] = [
    STRIDE_X, -STRIDE_X, STRIDE_Z, -STRIDE_Z, STRIDE_Y, -STRIDE_Y,
];

/// Per-worker leaf mask and BFS scratch. The vectors keep their capacity
/// between sections and modes; `Off` does no work at all.
#[derive(Default)]
pub(super) struct LeafMask {
    cells: Vec<Cell>,
    depth: Vec<u8>,
    queue: Vec<u16>,
    active: bool,
}

impl LeafMask {
    /// Classify the padded snapshot and, for the interior modes, measure how
    /// deep each leaf sits below the canopy surface.
    pub(super) fn prepare(
        &mut self,
        mode: LeafCullingMode,
        pad: &Padded,
        info: &[Info],
        region: &[[Option<&Chunk>; 3]; 3],
        base_y: i32,
    ) {
        self.active = mode != LeafCullingMode::Off;
        if !self.active {
            return;
        }
        // Most sections hold no leaves at all; do not classify 8,000 cells to
        // learn that. The centre is the only part this mesh can change.
        let mut leaves = false;
        'scan: for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    if info
                        .get(pad.blocks[pi(x, y, z)] as usize)
                        .is_some_and(|state| state.leaves)
                    {
                        leaves = true;
                        break 'scan;
                    }
                }
            }
        }
        if !leaves {
            self.active = false;
            return;
        }
        if self.cells.len() != P * P * P {
            self.cells.resize(P * P * P, Cell::Open);
        }
        for y in -B..16 + B {
            for z in -B..16 + B {
                for x in -B..16 + B {
                    let i = pi(x, y, z);
                    self.cells[i] = if !known(region, base_y, x, y, z) {
                        Cell::Unknown
                    } else {
                        match info.get(pad.blocks[i] as usize) {
                            None => Cell::Unknown,
                            Some(state) if state.leaves => Cell::Leaf,
                            Some(state) if state.opaque_cube => Cell::Opaque,
                            Some(_) => Cell::Open,
                        }
                    };
                }
            }
        }
        // Solid is a purely local six-neighbour test. Reserve the breadth-first
        // search for Solid Aggressive, whose one-open-side rule needs to know
        // whether a leaf is connected to the canopy surface through other
        // leaves.
        if mode == LeafCullingMode::SolidAggressive {
            self.depths();
        }
    }

    /// Multi-source BFS from leaves that touch an opening. Opaque neighbours do
    /// not expose a leaf: a leaf fully enclosed by stone is interior too.
    fn depths(&mut self) {
        if self.depth.len() != self.cells.len() {
            self.depth.resize(self.cells.len(), UNVISITED);
        } else {
            self.depth.fill(UNVISITED);
        }
        self.queue.clear();
        for y in -B..16 + B {
            for z in -B..16 + B {
                for x in -B..16 + B {
                    let i = pi(x, y, z);
                    if self.cells[i] != Cell::Leaf {
                        continue;
                    }
                    let touches_open = ADJACENT_STRIDES.iter().any(|&stride| {
                        let j = (i as isize + stride) as usize;
                        matches!(self.cells[j], Cell::Open | Cell::Unknown)
                    });
                    if touches_open {
                        self.depth[i] = 0;
                        self.queue.push(i as u16);
                    }
                }
            }
        }
        let mut head = 0;
        while head < self.queue.len() {
            let i = self.queue[head] as usize;
            head += 1;
            let next_depth = self.depth[i].saturating_add(1);
            for stride in ADJACENT_STRIDES {
                let j = (i as isize + stride) as usize;
                if self.cells[j] == Cell::Leaf && self.depth[j] == UNVISITED {
                    self.depth[j] = next_depth;
                    self.queue.push(j as u16);
                }
            }
        }
    }

    /// Whether the leaf at `(x, y, z)` may draw in the opaque layer.
    ///
    /// `Solid` wants a leaf with no open neighbour, which is exactly depth 1 or
    /// more. `Solid Aggressive` also takes the next layer of the shell: a leaf
    /// with a single open side joins the opaque core, so the cutout shell is
    /// one layer thinner. Unknown neighbours never qualify, however many sides
    /// they leave open. A leaf the search never reached sits inside a closed
    /// volume: interior for both.
    pub(super) fn interior(&self, mode: LeafCullingMode, x: i32, y: i32, z: i32) -> bool {
        if !self.active {
            return false;
        }
        let aggressive = match mode {
            LeafCullingMode::Solid => false,
            LeafCullingMode::SolidAggressive => true,
            _ => return false,
        };
        // A section boundary is not proof of a closed volume: the neighbouring
        // section may not be drawn yet, and its data is not ours to assume.
        if !(1..15).contains(&x) || !(1..15).contains(&y) || !(1..15).contains(&z) {
            return false;
        }
        let i = pi(x, y, z);
        if self.cells.get(i) != Some(&Cell::Leaf) {
            return false;
        }
        if !aggressive {
            return ADJACENT_STRIDES.iter().all(|&stride| {
                let j = (i as isize + stride) as usize;
                matches!(self.cells[j], Cell::Leaf | Cell::Opaque)
            });
        }
        let depth = self.depth[i];
        if depth == UNVISITED || depth >= 1 {
            return true;
        }
        let mut open = 0;
        for stride in ADJACENT_STRIDES {
            let j = (i as isize + stride) as usize;
            match self.cells[j] {
                Cell::Unknown => return false,
                Cell::Open => {
                    open += 1;
                    if open > 1 {
                        return false;
                    }
                }
                Cell::Leaf | Cell::Opaque => {}
            }
        }
        open == 1
    }
}

/// The cells this section's mesh owns. A face on this boundary belongs to the
/// neighbouring section as much as to this one; leaf culling leaves it alone.
#[inline]
pub(super) fn in_section(x: i32, y: i32, z: i32) -> bool {
    (0..16).contains(&x) && (0..16).contains(&y) && (0..16).contains(&z)
}

/// Whether the padded cell `(x, y, z)` has real block data. A missing chunk
/// does not count: `Padded::fill` copies the centre's edge column into it, which
/// would make a canopy look enclosed. A Y outside the column counts as unknown
/// for the same reason. The mask's outer shell is unknown too, so nothing is
/// inferred past the data the worker snapshotted.
fn known(region: &[[Option<&Chunk>; 3]; 3], base_y: i32, x: i32, y: i32, z: i32) -> bool {
    if x <= -B || x >= 16 + B - 1 || y <= -B || y >= 16 + B - 1 || z <= -B || z >= 16 + B - 1 {
        return false;
    }
    let chunk = region[((z + 16) >> 4) as usize][((x + 16) >> 4) as usize];
    chunk.is_some_and(|chunk| chunk.section_index(base_y + y).is_some())
}

#[cfg(test)]
impl LeafMask {
    /// Whether `Hollow` drops the face of the leaf at `(x, y, z)` toward `d`.
    /// Only a known leaf on the other side counts: open cells, partial models,
    /// missing neighbours and the mask's edge stay visible.
    pub(super) fn hides_face(
        &self,
        mode: LeafCullingMode,
        x: i32,
        y: i32,
        z: i32,
        d: horizon_core::geometry::Direction,
    ) -> bool {
        if !self.active || mode != LeafCullingMode::Hollow {
            return false;
        }
        let [dx, dy, dz] = d.normal();
        let (nx, ny, nz) = (x + dx, y + dy, z + dz);
        if !in_section(x, y, z) || !in_section(nx, ny, nz) {
            return false;
        }
        let i = pi(x, y, z);
        let ni = pi(nx, ny, nz);
        self.cells.get(i) == Some(&Cell::Leaf) && self.cells.get(ni) == Some(&Cell::Leaf)
    }

    #[inline]
    fn cell(&self, x: i32, y: i32, z: i32) -> Cell {
        checked(x, y, z)
            .and_then(|i| self.cells.get(i))
            .copied()
            .unwrap_or(Cell::Unknown)
    }
}

#[cfg(test)]
#[inline]
fn checked(x: i32, y: i32, z: i32) -> Option<usize> {
    if (-B..16 + B).contains(&x) && (-B..16 + B).contains(&y) && (-B..16 + B).contains(&z) {
        Some(pi(x, y, z))
    } else {
        None
    }
}

#[cfg(test)]
#[inline]
fn unpi(i: usize) -> (i32, i32, i32) {
    let x = (i % P) as i32 - B;
    let z = (i / P % P) as i32 - B;
    let y = (i / (P * P)) as i32 - B;
    (x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{blocks::RenderShape, graphics::LeafCullingMode};
    use horizon_core::geometry::Direction;
    use horizon_world::fluid::FluidState;
    use std::time::Instant;

    fn info(leaves: bool, opaque: bool) -> Info {
        Info {
            solid: opaque,
            full: opaque,
            permeable: false,
            shade: 1.0,
            emission: 0,
            tier: 0,
            emissive: false,
            fluid: FluidState::EMPTY,
            model: true,
            block_entity: false,
            tints: [super::super::Tint::None; 2],
            tint_count: 0,
            offset: None,
            legacy_solid: false,
            overlay: leaves,
            leaves,
            pane: false,
            opaque_cube: opaque,
            blocks_flow: false,
            ice: false,
            sturdy: 0,
            face: [0; 6],
            cover: [0.0; 6],
        }
    }

    /// State 0 is air, 1 leaves, 2 a full opaque cube, 3 a partial model.
    fn states() -> Vec<Info> {
        vec![
            info(false, false),
            info(true, false),
            info(false, true),
            info(false, false),
        ]
    }

    fn padded(fill: impl Fn(i32, i32, i32) -> u32) -> Padded {
        Padded {
            blocks: (0..P * P * P)
                .map(|i| {
                    let (x, y, z) = unpi(i);
                    fill(x, y, z)
                })
                .collect(),
            light: vec![0; P * P * P],
        }
    }

    fn cube(pad: &mut Padded, min: i32, max: i32, state: u32) {
        for y in min..=max {
            for z in min..=max {
                for x in min..=max {
                    pad.blocks[pi(x, y, z)] = state;
                }
            }
        }
    }

    fn loaded() -> Chunk {
        Chunk::empty(0, 0)
    }

    fn region(chunk: &Chunk) -> [[Option<&Chunk>; 3]; 3] {
        [[Some(chunk); 3]; 3]
    }

    fn built(mode: LeafCullingMode, pad: &Padded, states: &[Info], chunk: &Chunk) -> LeafMask {
        let mut mask = LeafMask::default();
        mask.prepare(mode, pad, states, &region(chunk), 0);
        mask
    }

    #[test]
    fn only_a_full_model_cube_is_opaque() {
        let combinations = [
            (RenderShape::Model, true, true, true),
            (RenderShape::Model, true, true, false),
            (RenderShape::Model, true, false, true),
            (RenderShape::Model, false, true, true),
            (RenderShape::Model, false, false, false),
            (RenderShape::Invisible, true, true, true),
            (RenderShape::Animated, true, true, true),
        ];
        let (shape, can_occlude, solid_render, full_collision) = combinations[0];
        assert!(opaque_cube(
            shape,
            can_occlude,
            solid_render,
            full_collision
        ));
        for &(shape, can_occlude, solid_render, full_collision) in &combinations[1..] {
            assert!(
                !opaque_cube(shape, can_occlude, solid_render, full_collision),
                "{shape:?} {can_occlude} {solid_render} {full_collision}"
            );
        }
    }

    #[test]
    fn solid_marks_the_cell_inside_a_three_cube() {
        let states = states();
        let mut pad = padded(|_, _, _| 0);
        cube(&mut pad, 7, 9, 1);
        let chunk = loaded();
        let solid = built(LeafCullingMode::Solid, &pad, &states, &chunk);
        assert!(!solid.interior(LeafCullingMode::Solid, 7, 8, 8));
        assert!(solid.interior(LeafCullingMode::Solid, 8, 8, 8));
        assert!(!solid.interior(LeafCullingMode::Solid, 8, 9, 8));
        // Aggressive also takes the six face-centre leaves, whose only open
        // side faces out of the cube. Corners and edges keep their cutout.
        let aggressive = built(LeafCullingMode::SolidAggressive, &pad, &states, &chunk);
        assert!(aggressive.interior(LeafCullingMode::SolidAggressive, 8, 8, 8));
        assert!(aggressive.interior(LeafCullingMode::SolidAggressive, 7, 8, 8));
        assert!(aggressive.interior(LeafCullingMode::SolidAggressive, 8, 8, 9));
        assert!(!aggressive.interior(LeafCullingMode::SolidAggressive, 7, 7, 8));
        assert!(!aggressive.interior(LeafCullingMode::SolidAggressive, 7, 7, 7));
    }

    #[test]
    fn aggressive_takes_the_next_shell_layer() {
        let states = states();
        let mut pad = padded(|_, _, _| 0);
        cube(&mut pad, 6, 10, 1);
        let chunk = loaded();
        let solid = built(LeafCullingMode::Solid, &pad, &states, &chunk);
        let aggressive = built(LeafCullingMode::SolidAggressive, &pad, &states, &chunk);
        let surface = |v: i32| v == 6 || v == 10;
        for [x, y, z] in [
            [6, 8, 8],
            [10, 8, 8],
            [6, 6, 8],
            [6, 6, 6],
            [7, 8, 8],
            [8, 8, 8],
            [7, 7, 7],
            [8, 8, 7],
        ] {
            let expected = !surface(x) && !surface(y) && !surface(z);
            assert_eq!(
                solid.interior(LeafCullingMode::Solid, x, y, z),
                expected,
                "solid {x} {y} {z}"
            );
            let open = [x, y, z].iter().filter(|v| surface(**v)).count();
            let expected = open <= 1;
            assert_eq!(
                aggressive.interior(LeafCullingMode::SolidAggressive, x, y, z),
                expected,
                "aggressive {x} {y} {z}"
            );
        }
    }

    #[test]
    fn hollow_hides_only_leaf_to_leaf_faces() {
        let states = states();
        let mut pad = padded(|_, _, _| 0);
        cube(&mut pad, 7, 9, 1);
        // Glass (3, a partial model), water and a slab are open: their leaves
        // must keep the face.
        pad.blocks[pi(10, 8, 8)] = 3;
        let chunk = loaded();
        let hollow = built(LeafCullingMode::Hollow, &pad, &states, &chunk);
        for d in Direction::ALL {
            assert!(
                hollow.hides_face(LeafCullingMode::Hollow, 8, 8, 8, d),
                "{d:?}"
            );
        }
        assert!(!hollow.hides_face(LeafCullingMode::Hollow, 9, 8, 8, Direction::East));
        assert!(!hollow.hides_face(LeafCullingMode::Hollow, 7, 8, 8, Direction::West));
        // The other modes never hide a quad.
        for mode in [
            LeafCullingMode::Off,
            LeafCullingMode::Solid,
            LeafCullingMode::SolidAggressive,
        ] {
            let mask = built(mode, &pad, &states, &chunk);
            assert!(!mask.hides_face(mode, 8, 8, 8, Direction::Up));
        }
    }

    #[test]
    fn a_cavity_keeps_its_faces_and_its_rim_visible() {
        let states = states();
        let mut pad = padded(|_, _, _| 0);
        cube(&mut pad, 6, 10, 1);
        pad.blocks[pi(8, 8, 8)] = 0;
        let chunk = loaded();
        let hollow = built(LeafCullingMode::Hollow, &pad, &states, &chunk);
        assert!(!hollow.hides_face(LeafCullingMode::Hollow, 7, 8, 8, Direction::East));
        assert!(!hollow.hides_face(LeafCullingMode::Hollow, 8, 8, 7, Direction::South));
        let solid = built(LeafCullingMode::Solid, &pad, &states, &chunk);
        assert!(!solid.interior(LeafCullingMode::Solid, 7, 8, 8));
        assert!(!solid.interior(LeafCullingMode::Solid, 6, 8, 8));
        // The corner away from the cavity is still two layers in.
        assert!(solid.interior(LeafCullingMode::Solid, 9, 9, 9));
    }

    #[test]
    fn an_opaque_neighbour_does_not_expose_a_leaf() {
        let states = states();
        let mut pad = padded(|_, _, _| 0);
        for (dx, dy, dz) in [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            pad.blocks[pi(8 + dx, 8 + dy, 8 + dz)] = 2;
        }
        pad.blocks[pi(8, 8, 8)] = 1;
        let chunk = loaded();
        for mode in [LeafCullingMode::Solid, LeafCullingMode::SolidAggressive] {
            let mask = built(mode, &pad, &states, &chunk);
            assert!(mask.interior(mode, 8, 8, 8), "{mode:?}");
        }
        let hollow = built(LeafCullingMode::Hollow, &pad, &states, &chunk);
        for d in Direction::ALL {
            assert!(
                !hollow.hides_face(LeafCullingMode::Hollow, 8, 8, 8, d),
                "{d:?}"
            );
        }
    }

    #[test]
    fn a_leaf_inside_a_closed_volume_is_interior() {
        let states = states();
        // A leaf cube inside opaque rock: the search has no opening to start
        // from, so every leaf stays unvisited and counts as interior.
        let mut pad = padded(|_, _, _| 2);
        cube(&mut pad, 7, 9, 1);
        let chunk = loaded();
        for mode in [LeafCullingMode::Solid, LeafCullingMode::SolidAggressive] {
            let mask = built(mode, &pad, &states, &chunk);
            if mode == LeafCullingMode::SolidAggressive {
                assert_eq!(mask.depth[pi(8, 8, 8)], UNVISITED);
            }
            assert!(mask.interior(mode, 8, 8, 8), "{mode:?}");
        }
    }

    #[test]
    fn missing_chunks_and_sections_are_unknown() {
        let states = states();
        let pad = padded(|_, _, _| 1);
        let chunk = loaded();
        let mut around = [[Some(&chunk); 3]; 3];
        around[1][2] = None; // east chunk
        let mut mask = LeafMask::default();
        mask.prepare(LeafCullingMode::Hollow, &pad, &states, &around, 0);
        // The leaf at the edge keeps its face toward the missing chunk, even
        // though `Padded::fill` copied the centre's edge leaf there.
        assert!(!mask.hides_face(LeafCullingMode::Hollow, 15, 8, 8, Direction::East));
        // A leaf one block inside is still between two known leaves.
        assert!(mask.hides_face(LeafCullingMode::Hollow, 14, 8, 8, Direction::East));

        // A section above the column is unknown too: the same leaf one
        // section up has no data, so it can never be interior.
        let short = Chunk::with_layout(0, 0, crate::chunk::ColumnLayout::new(0, 16).unwrap());
        let mut mask = LeafMask::default();
        mask.prepare(LeafCullingMode::Solid, &pad, &states, &region(&short), 16);
        assert!(!mask.interior(LeafCullingMode::Solid, 8, 8, 8));

        // The outer shell has no data beyond it.
        let mut mask = LeafMask::default();
        mask.prepare(LeafCullingMode::Hollow, &pad, &states, &region(&chunk), 0);
        assert_eq!(mask.cell(-B, 0, 0), Cell::Unknown);
        assert_eq!(mask.cell(16 + B - 1, 0, 0), Cell::Unknown);
        assert_eq!(mask.cell(0, 0, 0), Cell::Leaf);

        // A section boundary is never enclosed, even with every chunk loaded:
        // the neighbouring section may not have been meshed yet.
        let mut mask = LeafMask::default();
        mask.prepare(LeafCullingMode::Solid, &pad, &states, &region(&chunk), 0);
        assert!(!mask.interior(LeafCullingMode::Solid, 15, 8, 8));
        assert!(mask.interior(LeafCullingMode::Solid, 14, 8, 8));
    }

    #[test]
    fn off_does_no_work_and_hides_nothing() {
        let states = states();
        let mut pad = padded(|_, _, _| 0);
        cube(&mut pad, 7, 9, 1);
        let chunk = loaded();
        let mut mask = LeafMask::default();
        mask.prepare(LeafCullingMode::Off, &pad, &states, &region(&chunk), 0);
        assert!(!mask.active);
        assert!(mask.cells.is_empty());
        assert!(!mask.interior(LeafCullingMode::Off, 8, 8, 8));
        assert!(!mask.hides_face(LeafCullingMode::Off, 8, 8, 8, Direction::Up));
    }

    #[test]
    fn a_reused_mask_reclassifies_every_cell() {
        let states = states();
        let chunk = loaded();
        let mut mask = LeafMask::default();
        let mut leaves = padded(|_, _, _| 0);
        cube(&mut leaves, 7, 9, 1);
        mask.prepare(
            LeafCullingMode::Hollow,
            &leaves,
            &states,
            &region(&chunk),
            0,
        );
        assert_eq!(mask.cell(8, 8, 8), Cell::Leaf);
        let mut other = padded(|_, _, _| 0);
        cube(&mut other, 10, 12, 1);
        mask.prepare(LeafCullingMode::Hollow, &other, &states, &region(&chunk), 0);
        assert_eq!(mask.cell(8, 8, 8), Cell::Open);
        assert_eq!(mask.cell(11, 11, 11), Cell::Leaf);
    }

    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn mask_build_benchmark() {
        let states = states();
        let mut pad = padded(|x, y, z| {
            // Alternating leaf slabs make the search visit most of the volume.
            if (x / 2 + y / 2 + z / 2) % 3 == 0 {
                1
            } else {
                0
            }
        });
        cube(&mut pad, 6, 10, 1);
        let chunk = loaded();
        let region = region(&chunk);
        let iterations = 1000;
        for mode in [LeafCullingMode::Solid, LeafCullingMode::SolidAggressive] {
            let mut mask = LeafMask::default();
            let started = Instant::now();
            for _ in 0..iterations {
                mask.prepare(mode, &pad, &states, &region, 0);
            }
            let per = started.elapsed() / iterations;
            println!("leaf mask {mode:?}: {per:?} per section ({iterations} iterations)");
            assert!(mask.active);
            if mode == LeafCullingMode::SolidAggressive {
                assert_ne!(mask.depth[pi(8, 8, 8)], UNVISITED);
            }
        }
    }
}
