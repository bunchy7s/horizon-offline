//! "RUST EDITION" under the classic title screen's logo.
//!
//! Vanilla's `textures/gui/title/edition.png` reads "JAVA EDITION". Horizon
//! never ships Mojang's art, so the banner is rebuilt at load time from the
//! player's own texture: "EDITION" is kept column for column, the "T" is lifted
//! out of it, and R, U and S are Horizon's own glyphs (MIT, the rectangles
//! below), drawn the way the texture's letters are built (a face with darker
//! sides and a light bottom edge, a dark extrusion under it, a black outline
//! and a soft rim) in the shades sampled from the texture itself. Only
//! vanilla 26.3's exact texture is rebuilt: a resource pack's edition.png
//! stays as its author drew it.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;

/// The atlas sprite.
pub const SPRITE: &str = "gui/title/edition";
const WIDTH: usize = 512;
const HEIGHT: usize = 64;
/// FNV-1a over the size and RGBA pixels of 26.3's edition.png.
const VANILLA: u64 = 0x4249_0538_6dfb_181f;
/// "EDITION", outline included, starts at this column; left of it is "JAVA".
const EDITION_START: usize = 204;
/// The columns of the "T"'s body (face and extrusion) inside "EDITION".
const T_BODY: (usize, usize) = (335, 376);
/// Where the texture's letters put their faces, and how deep they are drawn.
const FACE_TOP: usize = 4;
const EXTRUSION: usize = 8;
const OUTLINE: usize = 3;
/// The face-to-face spacing between letters.
const LETTER_GAP: usize = 6;
/// The new glyphs' face width: "RUST" then spans "JAVA"'s columns to within
/// a texel, so "EDITION" stays where it is and the banner stays centred.
const GLYPH_WIDTH: usize = 42;
/// The first face column, as "JAVA"'s.
const FIRST_FACE: usize = 4;

/// Horizon's R, U and S: face rectangles `[x0, y0, x1, y1)` in texels from the
/// face's top-left, 42 wide and 39 tall, with 11-texel stems, 10-texel bars and
/// 7-texel notches for the rounded corners.
const R: &[[usize; 4]] = &[
    [0, 0, 11, 39],
    [0, 0, 35, 10],
    [31, 7, 42, 19],
    [0, 15, 35, 25],
    [31, 25, 42, 39],
];
const U: &[[usize; 4]] = &[[0, 0, 11, 32], [31, 0, 42, 32], [7, 29, 35, 39]];
const S: &[[usize; 4]] = &[
    [7, 0, 42, 10],
    [0, 7, 11, 19],
    [7, 15, 35, 25],
    [31, 21, 42, 32],
    [0, 29, 35, 39],
];

/// The banner rebuilt to read "RUST EDITION", or `None` when the texture is
/// not vanilla's (a resource pack's own).
pub fn rust_edition(width: u32, height: u32, pixels: &[u8]) -> Option<Vec<u8>> {
    (fingerprint(width, height, pixels) == VANILLA).then(|| compose(pixels))
}

fn fingerprint(width: u32, height: u32, pixels: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let bytes = width.to_le_bytes().into_iter().chain(height.to_le_bytes());
    for b in bytes.chain(pixels.iter().copied()) {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

type Rgba = [u8; 4];

fn lum(p: Rgba) -> u32 {
    (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> Rgba {
    let i = (y * WIDTH + x) * 4;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// The texture's shades: the most common colour in each brightness band.
struct Palette {
    highlight: Rgba,
    face: Rgba,
    rim: Rgba,
    extrusion: Rgba,
    edge: Rgba,
    outline: Rgba,
    soft: Rgba,
}

impl Palette {
    fn sample(pixels: &[u8]) -> Self {
        let mut bands: [HashMap<Rgba, u32>; 7] = Default::default();
        for &p in pixels.as_chunks::<4>().0 {
            let band = match (p[3], lum(p)) {
                (0, _) => continue,
                (255, 215..) => 0,
                (255, 170..) => 1,
                (255, 100..) => 2,
                (255, 45..) => 3,
                (255, 20..) => 4,
                (255, _) => 5,
                _ => 6,
            };
            *bands[band].entry(p).or_default() += 1;
        }
        let mode = |band: &HashMap<Rgba, u32>, fallback: Rgba| {
            band.iter()
                .max_by_key(|&(p, n)| (*n, std::cmp::Reverse(*p)))
                .map_or(fallback, |(p, _)| *p)
        };
        let outline = mode(&bands[5], [0, 0, 0, 255]);
        let face = mode(&bands[1], [200, 200, 200, 255]);
        Self {
            highlight: mode(&bands[0], face),
            face,
            rim: mode(&bands[2], face),
            extrusion: mode(&bands[3], outline),
            edge: mode(&bands[4], outline),
            outline,
            soft: mode(&bands[6], outline),
        }
    }
}

/// Faces of the new glyphs, positioned in the texture.
fn glyph_faces() -> Vec<bool> {
    let mut face = vec![false; WIDTH * HEIGHT];
    for (i, glyph) in [R, U, S].into_iter().enumerate() {
        let left = FIRST_FACE + i * (GLYPH_WIDTH + LETTER_GAP);
        for &[x0, y0, x1, y1] in glyph {
            for y in FACE_TOP + y0..FACE_TOP + y1 {
                for x in left + x0..left + x1 {
                    face[y * WIDTH + x] = true;
                }
            }
        }
    }
    face
}

/// Where the "T" lifted from "EDITION" goes: one letter gap after the S.
fn t_left() -> usize {
    FIRST_FACE + 3 * (GLYPH_WIDTH + LETTER_GAP)
}

/// Draw faces within `columns` the way the texture's letters are drawn.
fn draw_glyphs(out: &mut [u8], face: &[bool], palette: &Palette, columns: Range<usize>) {
    let at = |grid: &[bool], x: isize, y: isize| {
        (columns.start as isize..columns.end as isize).contains(&x)
            && (0..HEIGHT as isize).contains(&y)
            && grid[y as usize * WIDTH + x as usize]
    };
    let mut body = face.to_vec();
    for y in 0..HEIGHT {
        for x in columns.clone() {
            let (xi, yi) = (x as isize, y as isize);
            body[y * WIDTH + x] |= (1..=EXTRUSION as isize).any(|k| at(face, xi, yi - k));
        }
    }
    let reach = OUTLINE as isize + 1;
    for y in 0..HEIGHT {
        for x in columns.clone() {
            let (xi, yi) = (x as isize, y as isize);
            let colour = if at(face, xi, yi) {
                if !at(face, xi, yi + 1) {
                    palette.highlight
                } else if !at(face, xi - 1, yi) || !at(face, xi + 1, yi) {
                    palette.rim
                } else {
                    palette.face
                }
            } else if at(&body, xi, yi) {
                if at(face, xi, yi - 1) {
                    palette.rim
                } else if !at(&body, xi - 1, yi) || !at(&body, xi + 1, yi) || !at(&body, xi, yi + 1)
                {
                    palette.edge
                } else {
                    palette.extrusion
                }
            } else {
                let distance = (-reach..=reach)
                    .flat_map(|dy| (-reach..=reach).map(move |dx| (dx, dy)))
                    .filter(|&(dx, dy)| at(&body, xi + dx, yi + dy))
                    .map(|(dx, dy)| dx.abs().max(dy.abs()))
                    .min();
                match distance {
                    Some(d) if d <= OUTLINE as isize => palette.outline,
                    Some(_) => palette.soft,
                    None => continue,
                }
            };
            put(out, x, y, colour);
        }
    }
}

/// Paint `p` over what is there unless that is more opaque, or as opaque and
/// brighter (a letter's body wins over a neighbour's outline).
fn put(out: &mut [u8], x: usize, y: usize, p: Rgba) {
    let old = pixel(out, x, y);
    if (p[3], lum(p)) > (old[3], lum(old)) {
        out[(y * WIDTH + x) * 4..][..4].copy_from_slice(&p);
    }
}

/// Every pixel of "EDITION"'s "T": its body, and the outline pixels nearer to
/// it than to its neighbours.
fn t_pixels(pixels: &[u8]) -> Vec<(usize, usize)> {
    const NONE: u8 = 0;
    const T: u8 = 1;
    const OTHER: u8 = 2;
    let mut owner = vec![NONE; WIDTH * HEIGHT];
    let mut queue = VecDeque::new();
    for y in 0..HEIGHT {
        for x in EDITION_START..WIDTH {
            let p = pixel(pixels, x, y);
            if p[3] == 255 && lum(p) >= 30 {
                owner[y * WIDTH + x] = if (T_BODY.0..=T_BODY.1).contains(&x) {
                    T
                } else {
                    OTHER
                };
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let label = owner[y * WIDTH + x];
        for (dx, dy) in [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            let (nx, ny) = (x as isize + dx, y as isize + dy);
            if !(EDITION_START as isize..WIDTH as isize).contains(&nx)
                || !(0..HEIGHT as isize).contains(&ny)
            {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if owner[ny * WIDTH + nx] == NONE && pixel(pixels, nx, ny)[3] > 0 {
                owner[ny * WIDTH + nx] = label;
                queue.push_back((nx, ny));
            }
        }
    }
    (0..HEIGHT)
        .flat_map(|y| (EDITION_START..WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| owner[y * WIDTH + x] == T)
        .collect()
}

/// Close the pockets left of "EDITION" that no gap reaches from outside, as
/// the texture's letters fill their counters with the outline.
fn fill_counters(out: &mut [u8], outline: Rgba) {
    let open = |out: &[u8], x: usize, y: usize| pixel(out, x, y)[3] < 255;
    let mut seen = vec![false; WIDTH * HEIGHT];
    let mut queue = VecDeque::new();
    for y in 0..HEIGHT {
        for x in 0..EDITION_START {
            let border = x == 0 || y == 0 || x == EDITION_START - 1 || y == HEIGHT - 1;
            if border && open(out, x, y) {
                seen[y * WIDTH + x] = true;
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x as isize + dx, y as isize + dy);
            if !(0..EDITION_START as isize).contains(&nx) || !(0..HEIGHT as isize).contains(&ny) {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if !seen[ny * WIDTH + nx] && open(out, nx, ny) {
                seen[ny * WIDTH + nx] = true;
                queue.push_back((nx, ny));
            }
        }
    }
    for y in 0..HEIGHT {
        for x in 0..EDITION_START {
            if !seen[y * WIDTH + x] && open(out, x, y) {
                out[(y * WIDTH + x) * 4..][..4].copy_from_slice(&outline);
            }
        }
    }
}

/// Give every edge left of "EDITION" the soft rim the texture's outlines have
/// (the "T"'s right side shared its outline with the next letter).
fn soften_edges(out: &mut [u8], soft: Rgba) {
    let opaque = |out: &[u8], x: isize, y: isize| {
        (0..EDITION_START as isize).contains(&x)
            && (0..HEIGHT as isize).contains(&y)
            && pixel(out, x as usize, y as usize)[3] == 255
    };
    let mut rim = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..EDITION_START {
            let (xi, yi) = (x as isize, y as isize);
            let near = (-1..=1).any(|dy| (-1..=1).any(|dx| opaque(out, xi + dx, yi + dy)));
            if pixel(out, x, y)[3] == 0 && near {
                rim.push((x, y));
            }
        }
    }
    for (x, y) in rim {
        out[(y * WIDTH + x) * 4..][..4].copy_from_slice(&soft);
    }
}

/// "RUST" in place of "JAVA"; `pixels` is a 512x64 RGBA banner laid out as
/// vanilla's.
fn compose(pixels: &[u8]) -> Vec<u8> {
    let palette = Palette::sample(pixels);
    let mut out = pixels.to_vec();
    for row in out.as_chunks_mut::<{ WIDTH * 4 }>().0 {
        row[..EDITION_START * 4].fill(0);
    }
    draw_glyphs(&mut out, &glyph_faces(), &palette, 0..EDITION_START);
    let left = t_left();
    for (x, y) in t_pixels(pixels) {
        put(&mut out, x + left - T_BODY.0, y, pixel(pixels, x, y));
    }
    fill_counters(&mut out, palette.outline);
    soften_edges(&mut out, palette.soft);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACE: Rgba = [208, 197, 192, 255];

    /// A banner laid out as vanilla's, in made-up colours: every letter a
    /// solid block over its vanilla columns, with the font's shading.
    fn stand_in() -> Vec<u8> {
        // Vanilla's letter bodies (extrusion edge included) by column.
        let runs = [
            (4, 97),
            (101, 144),
            (147, 190),
            (208, 249),
            (256, 296),
            (303, 330),
            (335, 376),
            (381, 411),
            (413, 507),
        ];
        let mut face = vec![false; WIDTH * HEIGHT];
        for (x0, x1) in runs {
            for y in FACE_TOP..FACE_TOP + 39 {
                for x in x0..=x1 {
                    face[y * WIDTH + x] = true;
                }
            }
        }
        let palette = Palette {
            highlight: [226, 216, 213, 255],
            face: FACE,
            rim: [153, 147, 145, 255],
            extrusion: [61, 57, 56, 255],
            edge: [45, 42, 42, 255],
            outline: [0, 0, 0, 255],
            soft: [0, 0, 0, 244],
        };
        let mut out = vec![0; WIDTH * HEIGHT * 4];
        draw_glyphs(&mut out, &face, &palette, 0..WIDTH);
        // A colour only "JAVA" has, to see it go.
        out[(20 * WIDTH + 30) * 4..][..4].copy_from_slice(&[255, 0, 255, 255]);
        out
    }

    fn columns(pixels: &[u8], range: Range<usize>) -> Vec<u8> {
        pixels
            .as_chunks::<{ WIDTH * 4 }>()
            .0
            .iter()
            .flat_map(|row| row[range.start * 4..range.end * 4].to_vec())
            .collect()
    }

    /// Outlines left of "EDITION" keep their soft rim all round.
    fn assert_soft_rim(out: &[u8]) {
        for y in 1..HEIGHT - 1 {
            for x in 1..EDITION_START - 1 {
                if pixel(out, x, y)[3] == 255 {
                    let near = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)];
                    assert!(
                        near.iter().all(|&(nx, ny)| pixel(out, nx, ny)[3] > 0),
                        "{x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn rust_replaces_java_and_edition_is_untouched() {
        let source = stand_in();
        let out = compose(&source);
        assert_eq!(out.len(), WIDTH * HEIGHT * 4);
        assert_eq!(
            columns(&out, EDITION_START..WIDTH),
            columns(&source, EDITION_START..WIDTH)
        );
        assert!(!out.as_chunks::<4>().0.contains(&[255, 0, 255, 255]));
        // R, U and S: their strokes are faces, their notches and counters not.
        let lit = |x: usize, y: usize| lum(pixel(&out, x, y)) >= 100;
        let glyph = |i: usize, x: usize, y: usize| {
            lit(
                FIRST_FACE + i * (GLYPH_WIDTH + LETTER_GAP) + x,
                FACE_TOP + y,
            )
        };
        for (i, shape) in [R, U, S].into_iter().enumerate() {
            for &[x0, y0, x1, y1] in shape {
                assert!(glyph(i, (x0 + x1) / 2, (y0 + y1) / 2), "glyph {i}");
            }
        }
        assert!(!glyph(0, 38, 21), "R's notch between bowl and leg");
        assert!(!glyph(0, 20, 30), "R's open bottom");
        assert!(!glyph(1, 20, 5), "U's open top");
        assert!(!glyph(1, 2, 36), "U's rounded corner");
        assert!(!glyph(2, 2, 22), "S's lower-left notch");
        assert!(!glyph(2, 38, 17), "S's upper-right notch");
        // The T is EDITION's, moved.
        let shift = T_BODY.0 - t_left();
        for y in 0..HEIGHT {
            for x in T_BODY.0..=T_BODY.1 {
                let p = pixel(&source, x, y);
                if p[3] == 255 && lum(p) >= 30 {
                    assert_eq!(pixel(&out, x - shift, y), p, "T at {x},{y}");
                }
            }
        }
        assert_soft_rim(&out);
        // Nothing below the 14 GUI rows vanilla draws, and the word gap stays open.
        assert!(out[WIDTH * 55 * 4..].iter().all(|&b| b == 0));
        let gap = columns(&out, 195..EDITION_START);
        assert!(gap.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
        // Centred: "RUST" spans "JAVA"'s columns to within a texel.
        let used = |x: usize| (0..HEIGHT).any(|y| pixel(&out, x, y)[3] > 0);
        let last = (0..EDITION_START).rev().find(|&x| used(x)).unwrap();
        assert!(
            used(0) && (193..=195).contains(&last),
            "RUST ends at {last}"
        );
    }

    #[test]
    fn only_vanilla_is_rebuilt() {
        let source = stand_in();
        assert!(rust_edition(512, 64, &source).is_none());
    }

    /// Against the prepared vanilla resources when they are present.
    #[test]
    fn prepared_vanilla_edition() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION)
            .join("assets/minecraft/textures/gui/title/edition.png");
        let Ok((w, h, pixels)) = crate::hud::decode(&path) else {
            return;
        };
        let out = rust_edition(w, h, &pixels).expect("vanilla's edition.png");
        assert_eq!(
            columns(&out, EDITION_START..WIDTH),
            columns(&pixels, EDITION_START..WIDTH)
        );
        assert_eq!(Palette::sample(&pixels).face, FACE);
        assert_soft_rim(&out);
        let mut edited = pixels.clone();
        edited[3] ^= 1;
        assert!(rust_edition(w, h, &edited).is_none());
    }
}
