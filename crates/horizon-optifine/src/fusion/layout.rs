//! A connecting texture's image is a grid of tiles; its layout says which tiles a face draws
//! for the neighbours that join it (a [`crate::ctm::connect`] mask). Tiles are numbered row by
//! row from the top left of the grid.
//!
//! The tiles are FusionExamples' `templates/template <layout>.png`: each marks its joined sides
//! (and, where both sides beside a corner join, that corner) in green. `compact`'s template
//! names its 5 tiles, not their masks; which one a face takes is the wiki's GIF of the layout.

use crate::ctm::connect::{D, DL, DR, L, R, U, UL, UR, reduce};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// 5 tiles, a quarter of the face from each.
    Pieced,
    /// 16 tiles: the four sides only.
    Simple,
    /// 47 tiles: the sides and the corners.
    Full,
    /// 4 tiles in a column: alone, a column's top, middle and bottom.
    Vertical,
    /// 4 tiles in a row: alone, a row's left end, middle and right end.
    Horizontal,
    /// 5 tiles, a whole one per face.
    Compact,
    /// Edge pieces drawn over the face toward the neighbours that join it.
    Overlay,
}

/// What a face draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiles {
    One(u8),
    /// A tile per quarter of the face: top left, top right, bottom left, bottom right.
    Quarters([u8; 4]),
    /// [`Layout::overlay_tiles`] of this mask, layered; none when nothing joins.
    Overlay(u8),
}

/// Every side.
const SIDES: u8 = U | R | D | L;
const CORNERS: u8 = UR | DR | DL | UL;

/// The full template's tiles as the neighbours each joins. Its one empty cell is never drawn: a
/// lone corner, which [`reduce`] never leaves.
const EMPTY: u8 = UR;
const FULL: [u8; 48] = [
    0,
    R,
    R | L,
    L,
    R | D,
    D | L,
    U | R | D,
    R | D | L,
    D,
    R | DR | D,
    R | DR | D | DL | L,
    D | DL | L,
    U | R,
    U | L,
    U | R | L,
    U | D | L,
    U | D,
    U | UR | R | DR | D,
    SIDES | CORNERS,
    U | D | DL | L | UL,
    U | UR | R | D,
    R | DR | D | L,
    U | R | DR | D,
    R | D | DL | L,
    U,
    U | UR | R,
    U | UR | R | L | UL,
    U | L | UL,
    U | R | L | UL,
    U | D | DL | L,
    U | UR | R | L,
    U | D | L | UL,
    SIDES | UR | DL,
    SIDES,
    SIDES | UR | DR,
    SIDES | DR | DL,
    SIDES | DR,
    SIDES | DL,
    SIDES | UL | UR | DL,
    SIDES | UL | UR | DR,
    SIDES | UL | DR,
    EMPTY,
    SIDES | UL | UR,
    SIDES | UL | DL,
    SIDES | UR,
    SIDES | UL,
    SIDES | UL | DL | DR,
    SIDES | UR | DL | DR,
];

/// The simple template's 16 tiles as the sides each joins.
const SIMPLE: [u8; 16] = [
    0,
    SIDES,
    D,
    L,
    R | L,
    U | D,
    R,
    U,
    U | R | D,
    R | D | L,
    R | D,
    D | L,
    U | R | L,
    U | D | L,
    U | R,
    U | L,
];

/// `table`'s tile for each mask its entries can be: the inverse of the template.
const fn invert<const N: usize>(table: [u8; N]) -> [u8; 256] {
    let mut out = [0; 256];
    let mut i = 0;
    while i < N {
        if table[i] != EMPTY {
            out[table[i] as usize] = i as u8;
        }
        i += 1;
    }
    out
}

const FULL_INDEX: [u8; 256] = invert(FULL);
const SIMPLE_INDEX: [u8; 256] = invert(SIMPLE);

/// The overlay grid (6 x 3). On its left half an edge piece toward each side and a piece in
/// each corner; on its right the pieces for two and three sides joined through the corners
/// between them, and the ring for all eight neighbours.
const RING: u8 = 10;
/// Clockwise from the top: each side, and the corner after it.
const CYCLE: [(u8, u8); 4] = [(U, UR), (R, DR), (D, DL), (L, UL)];
/// By the side a run starts at, in `CYCLE`'s order: the piece for it alone, with the next side,
/// with the next two.
const EDGE: [u8; 4] = [13, 6, 1, 8];
const PAIR: [u8; 4] = [15, 3, 5, 17];
const TRIPLE: [u8; 4] = [9, 4, 11, 16];
/// The corner pieces, in `CYCLE`'s order of the corners.
const CORNER: [u8; 4] = [12, 0, 2, 14];

impl Layout {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "pieced" => Self::Pieced,
            "simple" => Self::Simple,
            "full" => Self::Full,
            "vertical" => Self::Vertical,
            "horizontal" => Self::Horizontal,
            "compact" => Self::Compact,
            "overlay" => Self::Overlay,
            _ => return None,
        })
    }

    /// The image's grid: columns, rows.
    pub fn grid(self) -> (u32, u32) {
        match self {
            Self::Pieced | Self::Compact => (5, 1),
            Self::Simple => (4, 4),
            Self::Full => (8, 6),
            Self::Vertical => (1, 4),
            Self::Horizontal => (4, 1),
            Self::Overlay => (6, 3),
        }
    }

    /// The tiles a face draws when the neighbours in `mask` join it.
    pub fn tiles(self, mask: u8) -> Tiles {
        let sides = mask & SIDES;
        let joined = |bits: u8| mask & bits == bits;
        let straight = |a, b| match (joined(a), joined(b)) {
            (false, false) => 0,
            (false, true) => 1,
            (true, true) => 2,
            (true, false) => 3,
        };
        match self {
            // The 5 tiles are `ctm_compact`'s: alone, joined all round, up and down, left and
            // right, all but the corner.
            Self::Pieced => Tiles::Quarters(crate::ctm::connect::compact_quarters(mask)),
            Self::Simple => Tiles::One(SIMPLE_INDEX[sides as usize]),
            Self::Full => Tiles::One(FULL_INDEX[reduce(mask) as usize]),
            // Alone, then the end that joins `b` (a column's top, a row's left end), the
            // middle, the other end.
            Self::Vertical => Tiles::One(straight(U, D)),
            Self::Horizontal => Tiles::One(straight(L, R)),
            // An end, the centre, a vertical and a horizontal section and a cross: joined
            // both ways, the centre only with every corner too.
            Self::Compact => Tiles::One(match (joined(L | R), joined(U | D)) {
                (true, true) if joined(CORNERS) => 1,
                (true, true) => 4,
                (false, true) => 2,
                (true, false) => 3,
                (false, false) => 0,
            }),
            Self::Overlay => Tiles::Overlay(mask),
        }
    }

    /// The overlay grid's tiles for `mask`, bottom first. Going round the face, each run of
    /// joined sides with joined corners between them is one piece; then each joined corner with
    /// neither side beside it. Unlike OptiFine's overlay, a corner matters between two joined
    /// sides: without it they are two pieces (the wiki's GIF of the layout).
    pub fn overlay_tiles(mask: u8) -> impl Iterator<Item = u8> {
        let mut out = [0; 4];
        let mut n = 0;
        let mut push = |tile| {
            out[n] = tile;
            n += 1;
        };
        let side = |i: usize| mask & CYCLE[i % 4].0 != 0;
        // Side `i` joins the next one clockwise.
        let linked = |i: usize| side(i) && side(i + 1) && mask & CYCLE[i % 4].1 != 0;
        if mask == 0xff {
            push(RING);
        } else {
            for start in (0..4).filter(|&i| side(i) && !linked(i + 3)) {
                let mut len = 1;
                while len < 4 && linked(start + len - 1) {
                    len += 1;
                }
                match len {
                    1 => push(EDGE[start]),
                    2 => push(PAIR[start]),
                    3 => push(TRIPLE[start]),
                    // Every side, one corner open: the grid has no piece for it. Two three-side
                    // pieces, overlapping on the middle two sides, keep every joined corner.
                    _ => {
                        push(TRIPLE[start]);
                        push(TRIPLE[(start + 1) % 4]);
                    }
                }
            }
            for (i, &(s, corner)) in CYCLE.iter().enumerate() {
                if mask & corner != 0 && mask & (s | CYCLE[(i + 1) % 4].0) == 0 {
                    push(CORNER[i]);
                }
            }
        }
        out.into_iter().take(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(layout: Layout, mask: u8) -> u8 {
        match layout.tiles(mask) {
            Tiles::One(t) => t,
            other => panic!("{other:?}"),
        }
    }

    /// The tiles `layout` draws over every mask.
    fn drawn(layout: Layout) -> std::collections::HashSet<u8> {
        (0..=255u8).map(|mask| one(layout, mask)).collect()
    }

    #[test]
    fn every_tile_is_drawn_for_some_mask() {
        // A mask a table lacks would fall to tile 0 and leave its own tile unused. The full
        // template's empty cell, (1, 5), is the one never drawn.
        let mut full: Vec<u8> = drawn(Layout::Full).into_iter().collect();
        full.sort_unstable();
        assert_eq!(full, (0..48).filter(|&t| t != 41).collect::<Vec<_>>());
        for layout in [
            Layout::Simple,
            Layout::Vertical,
            Layout::Horizontal,
            Layout::Compact,
        ] {
            let (w, h) = layout.grid();
            assert_eq!(drawn(layout).len() as u32, w * h, "{layout:?}");
        }
    }

    #[test]
    fn full_layout_reads_the_template() {
        // Alone: the top-left tile. A row's middle: the third.
        assert_eq!(one(Layout::Full, 0), 0);
        assert_eq!(one(Layout::Full, L | R), 2);
        // Joined all round: (2, 2); with no corners: (1, 4).
        assert_eq!(one(Layout::Full, 0xff), 18);
        assert_eq!(one(Layout::Full, SIDES), 33);
        // A corner whose sides don't both join changes nothing: up and the up-right corner is
        // the tile for up alone, (0, 3).
        assert_eq!(one(Layout::Full, U | UR), 24);
        assert_eq!(one(Layout::Full, U | UR | R), 25);
        // Every side and only the top-left corner: (5, 5).
        assert_eq!(one(Layout::Full, SIDES | UL), 45);
        // Every side and some corners, off the bottom two rows: opposite corners (0, 4) and
        // (0, 5); one corner (4, 4); three (6, 4).
        assert_eq!(one(Layout::Full, SIDES | UR | DL), 32);
        assert_eq!(one(Layout::Full, SIDES | UL | DR), 40);
        assert_eq!(one(Layout::Full, SIDES | DR), 36);
        assert_eq!(one(Layout::Full, SIDES | DL), 37);
        assert_eq!(one(Layout::Full, SIDES | UL | UR | DL), 38);
        // Three sides and one corner between them: (4, 3) and (6, 3).
        assert_eq!(one(Layout::Full, U | R | L | UL), 28);
        assert_eq!(one(Layout::Full, U | R | L | UR), 30);
    }

    #[test]
    fn simple_and_straight_layouts() {
        assert_eq!(one(Layout::Simple, D), 2);
        assert_eq!(one(Layout::Simple, U | L | UL), 15);
        // Corners don't count.
        assert_eq!(one(Layout::Simple, R | D | DR), 10);
        // The three-side tiles: (0, 2), (1, 2), (0, 3), (1, 3).
        assert_eq!(one(Layout::Simple, U | R | D), 8);
        assert_eq!(one(Layout::Simple, R | D | L), 9);
        assert_eq!(one(Layout::Simple, U | R | L), 12);
        assert_eq!(one(Layout::Simple, U | D | L), 13);
        // A column: bottom (joined up), middle, top.
        assert_eq!(one(Layout::Vertical, U), 3);
        assert_eq!(one(Layout::Vertical, U | D), 2);
        assert_eq!(one(Layout::Vertical, D), 1);
        assert_eq!(one(Layout::Vertical, L | R), 0);
        // A row: the left end joins right.
        assert_eq!(one(Layout::Horizontal, R), 1);
        assert_eq!(one(Layout::Horizontal, L | R | U), 2);
        assert_eq!(one(Layout::Horizontal, L), 3);
    }

    #[test]
    fn compact_picks_one_whole_tile() {
        // A row's end draws the end tile; its middle the horizontal section.
        assert_eq!(one(Layout::Compact, R), 0);
        assert_eq!(one(Layout::Compact, L | R), 3);
        // The top middle of a 3 x 3 square joins left, right and down: horizontal.
        assert_eq!(one(Layout::Compact, L | R | D | DL | DR), 3);
        assert_eq!(one(Layout::Compact, U | D | R), 2);
        // The square's middle, and a plus sign's.
        assert_eq!(one(Layout::Compact, 0xff), 1);
        assert_eq!(one(Layout::Compact, SIDES), 4);
        // Some corners aren't all of them: still the cross.
        assert_eq!(one(Layout::Compact, SIDES | UL | UR | DR), 4);
        // A square's corner joins two sides: an end.
        assert_eq!(one(Layout::Compact, R | D | DR), 0);
    }

    #[test]
    fn pieced_quarters() {
        // Joined right only: the left quarters are the lone tile, the right ones horizontal.
        assert_eq!(Layout::Pieced.tiles(R), Tiles::Quarters([0, 3, 0, 3]));
        // Joined up and right without the corner: the top-right quarter is the cross piece.
        assert_eq!(Layout::Pieced.tiles(U | R), Tiles::Quarters([2, 4, 0, 3]));
        assert_eq!(
            Layout::Pieced.tiles(U | R | UR),
            Tiles::Quarters([2, 1, 0, 3])
        );
    }

    #[test]
    fn overlay_layers_edges_and_corners() {
        let cells = |mask| Layout::overlay_tiles(mask).collect::<Vec<_>>();
        assert_eq!(cells(0), [0u8; 0]);
        // Sand below: the bottom edge piece, (1, 0); to the right: (0, 1).
        assert_eq!(cells(D), [1]);
        assert_eq!(cells(R), [6]);
        // Sand all round: the ring, (4, 1).
        assert_eq!(cells(0xff), [10]);
        // Two sides and the corner between them: one L-shaped piece off the right half.
        assert_eq!(cells(L | D | DL), [5]);
        assert_eq!(cells(R | D | DR), [3]);
        assert_eq!(cells(U | R | UR), [15]);
        assert_eq!(cells(U | L | UL), [17]);
        // Without the corner: two edge pieces.
        assert_eq!(cells(R | D), [6, 1]);
        // Three sides and both corners between them.
        assert_eq!(cells(R | D | L | DR | DL), [4]);
        assert_eq!(cells(U | R | D | UR | DR), [9]);
        assert_eq!(cells(U | D | L | UL | DL), [11]);
        assert_eq!(cells(U | R | L | UL | UR), [16]);
        // The wiki's GIF: sand up, up-left, left and right, not up-right. The up-left piece,
        // (5, 2), and the right edge, (0, 1).
        assert_eq!(cells(U | UL | L | R), [6, 17]);
        // Every side and no corner: four edges. Every side but the top-left corner: no
        // template or GIF shows it, so this pins `overlay_tiles`' own choice, the pieces for
        // up-right-down and right-down-left.
        assert_eq!(cells(U | R | D | L), [13, 6, 1, 8]);
        assert_eq!(cells(!UL), [9, 4]);
        // A lone corner each.
        assert_eq!(cells(DL), [2]);
        assert_eq!(cells(UL), [14]);
        // Above and below: two edge pieces.
        assert_eq!(cells(U | D), [13, 1]);
        // A column of sand to the left, and sand at the right corners only: the left edge,
        // then the top-right and bottom-right corner pieces.
        assert_eq!(cells(UL | L | DL | UR | DR), [8, 12, 0]);
    }
}
