//! `BlockPattern`: a grid of block predicates found in the world in any of
//! its 24 orientations (upright, upside down, lying flat, turned sideways).
//! The wither is built from these.
use horizon_core::geometry::Direction;

/// A grid of predicate keys: `aisles[z][y]` is a row whose chars are the `x`
/// cells (`BlockPatternBuilder.aisle`). A space matches anything.
#[derive(Clone, Copy, Debug)]
pub struct BlockPattern {
    pub aisles: &'static [&'static [&'static str]],
}

/// `WitherSkullBlock.getOrCreateWitherFull`: '^' a wither skeleton skull,
/// '#' a wither summon base block, '~' air.
pub const WITHER_FULL: BlockPattern = BlockPattern {
    aisles: &[&["^^^", "###", "~#~"]],
};
/// `WitherSkullBlock.getOrCreateWitherBase`.
pub const WITHER_BASE: BlockPattern = BlockPattern {
    aisles: &[&["   ", "###", "~#~"]],
};
/// `BlockPattern.BlockPatternMatch`: where a pattern was found and how it is turned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternMatch {
    pub front_top_left: [i32; 3],
    pub forwards: Direction,
    pub up: Direction,
    pub width: i32,
    pub height: i32,
    pub depth: i32,
}

impl PatternMatch {
    /// `BlockPatternMatch.getBlock(x, y, z)`'s position.
    pub fn block(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        translate_and_rotate(self.front_top_left, self.forwards, self.up, x, y, z)
    }
    /// The front layer, column by column (`CarvedPumpkinBlock.clearPatternBlocks`
    /// and `updatePatternBlocks`).
    pub fn front_cells(&self) -> Vec<[i32; 3]> {
        (0..self.width)
            .flat_map(|x| (0..self.height).map(move |y| (x, y)))
            .map(|(x, y)| self.block(x, y, 0))
            .collect()
    }
}

/// `BlockPattern.translateAndRotate`: `x` runs along forwards × up, `y` down
/// against up, `z` along forwards.
pub fn translate_and_rotate(
    pos: [i32; 3],
    forwards: Direction,
    up: Direction,
    x: i32,
    y: i32,
    z: i32,
) -> [i32; 3] {
    let f = forwards.normal();
    let u = up.normal();
    let right = [
        f[1] * u[2] - f[2] * u[1],
        f[2] * u[0] - f[0] * u[2],
        f[0] * u[1] - f[1] * u[0],
    ];
    std::array::from_fn(|i| pos[i] - u[i] * y + right[i] * x + f[i] * z)
}

impl BlockPattern {
    pub fn width(&self) -> i32 {
        self.aisles
            .first()
            .and_then(|a| a.first())
            .map_or(0, |row| row.len() as i32)
    }
    pub fn height(&self) -> i32 {
        self.aisles.first().map_or(0, |a| a.len() as i32)
    }
    pub fn depth(&self) -> i32 {
        self.aisles.len() as i32
    }
    /// `BlockPattern.matches`: every cell of the pattern with its front top
    /// left at `corner`, turned by `forwards` and `up`, passes `test(key, pos)`.
    pub fn matches(
        &self,
        corner: [i32; 3],
        forwards: Direction,
        up: Direction,
        test: &mut impl FnMut(char, [i32; 3]) -> bool,
    ) -> Option<PatternMatch> {
        for x in 0..self.width() {
            for y in 0..self.height() {
                for z in 0..self.depth() {
                    let key = self.aisles[z as usize][y as usize]
                        .as_bytes()
                        .get(x as usize)
                        .map_or(' ', |&b| b as char);
                    if key != ' ' && !test(key, translate_and_rotate(corner, forwards, up, x, y, z))
                    {
                        return None;
                    }
                }
            }
        }
        Some(PatternMatch {
            front_top_left: corner,
            forwards,
            up,
            width: self.width(),
            height: self.height(),
            depth: self.depth(),
        })
    }
    /// `BlockPattern.find`: the first match whose front top left corner lies
    /// in the cube from `pos` to `pos + size - 1` (x fastest, then y, then z),
    /// trying every forwards and up pair (`Direction.values()` order) that is
    /// not along one axis.
    pub fn find(
        &self,
        pos: [i32; 3],
        mut test: impl FnMut(char, [i32; 3]) -> bool,
    ) -> Option<PatternMatch> {
        let size = self.width().max(self.height()).max(self.depth());
        for dz in 0..size {
            for dy in 0..size {
                for dx in 0..size {
                    let corner = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
                    for forwards in Direction::ALL {
                        for up in Direction::ALL {
                            if up == forwards || up == forwards.opposite() {
                                continue;
                            }
                            if let Some(m) = self.matches(corner, forwards, up, &mut test) {
                                return Some(m);
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A world of named blocks; anything not listed is air.
    fn world(blocks: &[([i32; 3], char)]) -> HashMap<[i32; 3], char> {
        blocks.iter().copied().collect()
    }
    /// Each key tests for its own char in the fixture, '~' for air.
    fn finder(w: &HashMap<[i32; 3], char>) -> impl FnMut(char, [i32; 3]) -> bool + '_ {
        move |key, p| match key {
            '~' => !w.contains_key(&p),
            k => w.get(&p) == Some(&k),
        }
    }
    /// The wither T laid out along `right` and `down` from `top_left`.
    fn wither(top_left: [i32; 3], right: [i32; 3], down: [i32; 3]) -> Vec<([i32; 3], char)> {
        let at = |x: i32, y: i32| std::array::from_fn(|i| top_left[i] + right[i] * x + down[i] * y);
        let mut v: Vec<_> = (0..3).map(|x| (at(x, 0), '^')).collect();
        v.extend((0..3).map(|x| (at(x, 1), '#')));
        v.push((at(1, 2), '#'));
        v
    }
    fn found(blocks: &[([i32; 3], char)], placed: [i32; 3]) -> Option<PatternMatch> {
        let w = world(blocks);
        WITHER_FULL.find(placed, finder(&w))
    }

    #[test]
    fn every_axis_of_rotation_is_tried() {
        // Each forwards/up pair puts x, y and z along three different axes.
        for f in Direction::ALL {
            for u in Direction::ALL {
                if u == f || u == f.opposite() {
                    continue;
                }
                let cells: Vec<_> = (0..2)
                    .flat_map(|x| (0..2).flat_map(move |y| (0..2).map(move |z| (x, y, z))))
                    .map(|(x, y, z)| translate_and_rotate([0; 3], f, u, x, y, z))
                    .collect();
                let mut unique = cells.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(unique.len(), 8, "{f:?} {u:?}");
            }
        }
    }

    #[test]
    fn wither_builds_in_any_orientation() {
        let x = [1, 0, 0];
        let y = [0, 1, 0];
        let z = [0, 0, 1];
        let neg = |v: [i32; 3]| v.map(|c| -c);
        // (right, down) for each layout: upright along X and Z, upside
        // down, lying flat both ways, and the T turned on its side.
        let layouts = [
            ("upright along x", x, neg(y)),
            ("upright along z", z, neg(y)),
            ("upside down", x, y),
            ("flat, skulls north", x, z),
            ("flat, skulls east", z, neg(x)),
            ("sideways, skulls west", y, x),
            ("sideways, skulls south", y, neg(z)),
        ];
        for (name, right, down) in layouts {
            let top_left = [10, 64, -5];
            let blocks = wither(top_left, right, down);
            for skull in 0..3 {
                let placed: [i32; 3] = std::array::from_fn(|i| top_left[i] + right[i] * skull);
                let m = found(&blocks, placed).unwrap_or_else(|| panic!("{name}, skull {skull}"));
                // The wither stands on the middle of the bottom row.
                let middle: [i32; 3] =
                    std::array::from_fn(|i| top_left[i] + right[i] + down[i] * 2);
                assert_eq!(m.block(1, 2, 0), middle, "{name}");
                let mut cells = m.front_cells();
                cells.sort();
                let mut expected: Vec<[i32; 3]> = (0..3)
                    .flat_map(|c| (0..3).map(move |r| (c, r)))
                    .map(|(c, r)| std::array::from_fn(|i| top_left[i] + right[i] * c + down[i] * r))
                    .collect();
                expected.sort();
                assert_eq!(cells, expected, "{name}");
            }
        }
    }

    #[test]
    fn wither_faces_along_the_pattern_normal() {
        // WitherSkullBlock.checkSpawn turns the wither by forwards' axis,
        // the normal of the T: an upright T along X faces along Z.
        let along_x = wither([0, 66, 0], [1, 0, 0], [0, -1, 0]);
        let m = found(&along_x, [0, 66, 0]).unwrap();
        assert!(matches!(m.forwards, Direction::North | Direction::South));
        let along_z = wither([0, 66, 0], [0, 0, 1], [0, -1, 0]);
        let m = found(&along_z, [0, 66, 0]).unwrap();
        assert!(matches!(m.forwards, Direction::West | Direction::East));
    }

    #[test]
    fn wither_needs_air_beside_the_leg() {
        let mut blocks = wither([0, 66, 0], [1, 0, 0], [0, -1, 0]);
        blocks.push(([0, 64, 0], 's'));
        assert_eq!(found(&blocks, [1, 66, 0]), None);
    }

    #[test]
    fn the_base_pattern_ignores_the_skull_row() {
        let base = world(&[
            ([0, 64, 0], '#'),
            ([1, 64, 0], '#'),
            ([2, 64, 0], '#'),
            ([1, 63, 0], '#'),
        ]);
        assert!(WITHER_BASE.find([0, 65, 0], finder(&base)).is_some());
        assert!(WITHER_FULL.find([0, 65, 0], finder(&base)).is_none());
    }
}
