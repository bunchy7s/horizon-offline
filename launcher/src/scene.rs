//! The pixel sunset behind the glass look: a port of the website's `HorizonScene`
//! (horizon-site `components/horizon-scene.tsx`, as the launcher mockup draws it with
//! `wideSun={{ x: 0.72, scale: 1.1, rise: 0.1 }}`), with its seeds, its random draws in the same
//! order and its colour sums, so the same window size gives the same picture.
//!
//! The website paints two canvases each frame. Here the parts that never move (the sky's
//! gradients, the sun's halo, the three ridges with their trees and fog) are baked into images
//! once per window size; what moves is a few hundred quads a frame: the stars, a meteor, the
//! clouds and the sun in runs of one colour, and the fireflies. The scene is its own view, so a
//! frame of it doesn't build the launcher's pages again: GPUI Fast draws them from the last frame.
//!
//! [`Backdrop`] is the same picture blurred, for the glass plates (`glass.rs`): GPUI has no
//! backdrop filter, but the only thing under a plate is this scene.
use crate::art;
use crate::sky::{motion, Clock, Motion};
use gpui::{
    canvas, div, fill, img, point, prelude::*, px, size, Bounds, Context, Corners, Hsla, Pixels,
    RenderImage, Rgba, Window,
};
use std::f64::consts::PI;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SKY: [u32; 15] = [
    0x0a0919, 0x0f0c28, 0x151136, 0x1d1444, 0x281850, 0x361b5b, 0x481e66, 0x5d236e, 0x752873,
    0x902f76, 0xab3a75, 0xc64a71, 0xde5f6a, 0xef7c63, 0xf89d62,
];
const SKY_STEPS: usize = SKY.len() - 4;
const SUN: [u32; 7] = [
    0xfff5d2, 0xffe4a0, 0xffca78, 0xffac5f, 0xff8c58, 0xfb6c62, 0xee5277,
];
const HALO: u32 = 0xff8d6a;
const FOG: u32 = 0xe0697c;
const RIM: u32 = 0xffb27c;
const CLOUD_HIGH: u32 = 0x5f2c7c;
const CLOUD_LOW: u32 = 0xc95985;
const CLOUD_TOP: u32 = 0xf3a6bd;
const FIREFLY: u32 = 0xffd98c;
const STAR_COOL: u32 = 0xd2c8ff;
const STAR_WARM: u32 = 0xfff3e4;
const MARGIN: i32 = 20;
/// The mockup's sun: right of centre, a little bigger and higher (`wideSun`), on wide windows.
const WIDE_SUN: (f64, f64, f64) = (0.72, 1.1, 0.1);
/// The website draws a frame when 31 ms have passed since the last.
const TICK: Duration = Duration::from_millis(31);
/// How long the scene keeps moving after the window loses focus.
const UNFOCUSED: Duration = Duration::from_secs(10);

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn smoothstep(a: f64, b: f64, v: f64) -> f64 {
    let t = clamp((v - a) / (b - a), 0., 1.);
    t * t * (3. - 2. * t)
}

/// JavaScript's `Math.round`: halves go up.
fn round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// The website's `mix`: each channel `a + (b - a) * t`, cut to an integer as its bit operations do.
fn mix(a: u32, b: u32, t: f64) -> u32 {
    [16, 8, 0].iter().fold(0, |out, &s| {
        let (x, y) = (((a >> s) & 255) as f64, ((b >> s) & 255) as f64);
        out | ((x + (y - x) * t) as i64 as u32 & 255) << s
    })
}

/// The website's `rng`: mulberry32.
struct Rng(i32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let s = self.0;
        let mut t = (s ^ ((s as u32) >> 15) as i32).wrapping_mul(1 | s);
        t = t.wrapping_add((t ^ ((t as u32) >> 7) as i32).wrapping_mul(61 | t)) ^ t;
        ((t ^ ((t as u32) >> 14) as i32) as u32) as f64 / 4294967296.
    }
}

/// The website's `noise1d`: four octaves of smoothed value noise over 256 random lattice points,
/// kept as 32-bit floats like its `Float32Array`.
struct Noise([f32; 256]);

impl Noise {
    fn new(seed: i32) -> Noise {
        let mut rng = Rng(seed);
        Noise(std::array::from_fn(|_| rng.next() as f32))
    }

    fn at(&self, x: f64) -> f64 {
        let (mut value, mut amp, mut freq, mut norm) = (0., 0.5, 1., 0.);
        for _ in 0..4 {
            let p = x * freq;
            let i = p.floor();
            let s = p - i;
            let i = i as i64;
            let a = self.0[(i & 255) as usize] as f64;
            let b = self.0[((i + 1) & 255) as usize] as f64;
            value += (a + (b - a) * s * s * (3. - 2. * s)) * amp;
            norm += amp;
            amp *= 0.5;
            freq *= 2.1;
        }
        value / norm
    }
}

const TREES: [&[&str]; 3] = [
    &[
        "..###..", ".#####.", "#######", "#######", ".#####.", "...#...", "...#...",
    ],
    &[
        "..#..", "..#..", ".###.", "..#..", ".###.", "#####", "..#..", "..#..",
    ],
    &[".###.", "#####", "#####", "..#..", "..#.."],
];

struct Star {
    x: f64,
    y: f64,
    size: f64,
    b: f64,
    phase: f64,
    speed: f64,
    sparkle: bool,
    cool: bool,
}

struct Cloud {
    x0: f64,
    y: i32,
    w: i32,
    h: i32,
    /// 1 body, 2 the lit bottom row, 3 the top edge.
    mask: Vec<u8>,
    speed: f64,
    depth: f64,
    body: u32,
    rim_base: f64,
}

struct Ridge {
    top: Vec<i32>,
    depth: f64,
    color: u32,
    rim: u32,
    rim_strength: f64,
    /// Trees standing on it: their column and sprite.
    trees: Vec<(i32, usize)>,
}

struct Firefly {
    x: f64,
    y: f64,
    phase: f64,
    blink: f64,
}

/// One window size's scene, as the website's `buildScene` makes it. Lengths are texels (`px`
/// logical pixels square) except the stars', which are logical pixels.
pub struct Model {
    width: f64,
    px: f64,
    w: i32,
    h: i32,
    horizon: i32,
    fog_top: i32,
    fog_bottom: i32,
    sun: (i32, i32, i32),
    glow: Vec<f64>,
    stars: Vec<Star>,
    clouds: Vec<Cloud>,
    ridges: Vec<Ridge>,
    flies: Vec<Firefly>,
}

fn make_cloud(rand: &mut Rng, w: i32, h: i32) -> Vec<u8> {
    let mut mask = vec![0u8; (w * h).max(0) as usize];
    let mut lobe = |rand: &mut Rng, from: i32, to: i32| {
        let (mut left, mut right, mut y) = (from, to, h);
        while y > 0 && right - left > 3 {
            let mut k = y.min(1 + (rand.next() * 2.).floor() as i32);
            while k > 0 {
                y -= 1;
                for x in left..right {
                    mask[(y * w + x) as usize] = 1;
                }
                k -= 1;
            }
            let span = (right - left) as f64;
            left += 1f64.max(round(span * (0.06 + rand.next() * 0.2))) as i32;
            right -= 1f64.max(round(span * (0.06 + rand.next() * 0.2))) as i32;
        }
    };
    lobe(rand, 0, w);
    if w > 24 && rand.next() < 0.75 {
        let a = round(w as f64 * (0.1 + rand.next() * 0.4)) as i32;
        let to = w.min(a + round(w as f64 * (0.35 + rand.next() * 0.2)) as i32);
        lobe(rand, a, to);
    }
    for i in 0..(w * h) as usize {
        if mask[i] == 0 {
            continue;
        }
        if i >= ((h - 1) * w) as usize {
            mask[i] = 2;
        } else if i < w as usize || mask[i - w as usize] == 0 {
            mask[i] = 3;
        }
    }
    mask
}

impl Model {
    /// The scene for a window `width` × `height` logical pixels, its land `clamp(30 % of the
    /// height, 180, 320)` high as the website's default.
    pub fn new(width: f64, height: f64) -> Model {
        let land_height = clamp(height * 0.3, 180., 320.);
        let (sun_x, sun_scale, sun_rise) = if width >= 1024. {
            WIDE_SUN
        } else {
            (0.5, 1., 0.)
        };
        let px = clamp(round(width / 340.), 3., 6.);
        let w = (width / px).ceil() as i32;
        let h = (height / px).ceil() as i32;
        let land = land_height / px;
        let horizon = round(h as f64 - land * 0.55) as i32;
        let r = round(land * 0.4 * sun_scale) as i32;
        let sun = (
            round(w as f64 * sun_x) as i32,
            round(horizon as f64 - r as f64 * (0.45 + sun_rise)) as i32,
            r,
        );
        let fog_top = round(horizon as f64 - land * 0.18) as i32;
        let glow = (0..w)
            .map(|x| {
                let d = (x - sun.0) as f64 / (w as f64 * 0.3);
                (-d * d).exp()
            })
            .collect::<Vec<_>>();
        let sky_at = |y: f64| {
            let f = clamp(y / horizon as f64, 0., 1.).powf(2.3) * SKY_STEPS as f64;
            let i = f.floor() as usize;
            mix(SKY[i], SKY[(i + 1).min(SKY.len() - 1)], f - i as f64)
        };

        let mut rand = Rng(7);
        let sky_height = horizon as f64 * px;
        let star_count = round(width * sky_height / 5200.) as usize;
        let mut stars = Vec::with_capacity(star_count);
        for _ in 0..star_count {
            let x = rand.next() * width;
            let y = 2. + rand.next().powf(1.8) * sky_height * 0.75;
            let size = if rand.next() < 0.15 {
                1.8
            } else {
                1. + rand.next() * 0.4
            };
            let b = if rand.next() < 0.1 {
                0.8 + rand.next() * 0.2
            } else {
                0.25 + rand.next() * 0.4
            };
            stars.push(Star {
                x,
                y,
                size,
                b,
                phase: rand.next() * PI * 2.,
                speed: 0.5 + rand.next() * 2.,
                sparkle: rand.next() < 0.06,
                cool: rand.next() < 0.35,
            });
        }

        let mut clouds = Vec::new();
        let mut add_cloud = |rand: &mut Rng, y: f64, cw: f64, ch: f64, speed: f64, depth: f64| {
            let t = clamp(
                (y - horizon as f64 * 0.45) / (horizon as f64 * 0.55),
                0.,
                1.,
            );
            let x0 = rand.next() * (w as f64 + cw);
            let (cw, ch) = (round(cw) as i32, round(ch) as i32);
            clouds.push(Cloud {
                x0,
                y: round(y) as i32,
                w: cw,
                h: ch,
                mask: make_cloud(rand, cw, ch),
                speed,
                depth,
                body: mix(sky_at(y), mix(CLOUD_HIGH, CLOUD_LOW, t * t), 0.55),
                rim_base: 0.15 + t * 0.35,
            });
        };
        for _ in 0..3 {
            let cw = (120. + rand.next() * 140.) / px;
            let y = horizon as f64 - land * (0.45 + rand.next() * 0.25);
            let ch = cw * (0.2 + rand.next() * 0.06);
            let speed = 0.5 + rand.next() * 0.8;
            add_cloud(&mut rand, y, cw, ch, speed, 0.4);
        }
        for _ in 0..5 {
            let y = horizon as f64 - 5. - rand.next() * land * 0.4;
            let cw = w as f64 * (0.08 + rand.next() * 0.12);
            let ch = 3. + rand.next() * 3.;
            let speed = 1. + rand.next() * 1.6;
            add_cloud(&mut rand, y, cw, ch, speed, 0.8);
        }

        let cols = w + MARGIN * 2;
        #[allow(clippy::too_many_arguments)]
        let ridge = |seed: i32,
                     base: f64,
                     amp: f64,
                     block: i32,
                     feature: f64,
                     valley: f64,
                     depth: f64,
                     color: u32,
                     rim: u32,
                     rim_strength: f64| {
            let n = Noise::new(seed);
            let mut top = vec![0i32; cols as usize];
            let mut i = 0;
            while i < cols {
                let x = (i - MARGIN) as f64 + block as f64 / 2.;
                let d = (x - sun.0 as f64).abs() / (w as f64 * 0.5);
                let env = valley + (1. - valley) * (d * d * 1.8).min(1.);
                let v = clamp((n.at(x * px / feature) - 0.2) / 0.6, 0., 1.);
                let y = round((base - amp * env * v) / block as f64) as i32 * block;
                for k in 0..block {
                    if i + k < cols {
                        top[(i + k) as usize] = y;
                    }
                }
                i += block;
            }
            Ridge {
                top,
                depth,
                color,
                rim,
                rim_strength,
                trees: Vec::new(),
            }
        };
        let hz = horizon as f64;
        let mut ridges = vec![
            ridge(
                11,
                hz,
                land * 0.3,
                2,
                170.,
                0.2,
                0.25,
                0x3f1f5c,
                0xc45a7f,
                1.,
            ),
            ridge(
                23,
                hz + land * 0.22,
                land * 0.36,
                3,
                130.,
                0.35,
                0.55,
                0x25123e,
                0x7a3068,
                0.8,
            ),
            ridge(
                37,
                hz + land * 0.5,
                land * 0.38,
                4,
                100.,
                0.5,
                1.,
                0x100a1f,
                0x2c1640,
                0.6,
            ),
        ];
        let mut col = MARGIN + (rand.next() * 12.).floor() as i32;
        while col < cols - 8 {
            let sprite = (rand.next() * TREES.len() as f64).floor() as usize;
            ridges[2].trees.push((col, sprite));
            col += 14 + (rand.next() * 30.).floor() as i32;
        }

        let flies = (0..round(w as f64 / 36.) as usize)
            .map(|_| Firefly {
                x: rand.next() * w as f64,
                y: hz + land * (0.3 + rand.next() * 0.6),
                phase: rand.next() * PI * 2.,
                blink: 0.6 + rand.next() * 0.9,
            })
            .collect();

        Model {
            width,
            px,
            w,
            h,
            horizon,
            fog_top,
            fog_bottom: round(hz + land * 0.26) as i32,
            sun,
            glow,
            stars,
            clouds,
            ridges,
            flies,
        }
    }

    /// The static sky (`paintStaticSky`) at a point in logical pixels, as straight RGB in 0..=1:
    /// the gradient to the horizon, the sun's glow on it and the haze over the land.
    fn sky_rgb(&self, x: f64, y: f64) -> [f64; 3] {
        let horizon = self.horizon as f64 * self.px;
        let t = clamp(y / horizon, 0., 1.);
        let stop = |i: usize| (i as f64 / SKY_STEPS as f64).powf(1. / 2.3);
        let mut i = 0;
        while i + 1 < SKY_STEPS && t > stop(i + 1) {
            i += 1;
        }
        let f = clamp((t - stop(i)) / (stop(i + 1) - stop(i)), 0., 1.);
        let mut c = lerp(rgb(SKY[i]), rgb(SKY[i + 1]), f);
        // The glow: a radial gradient squeezed to an ellipse `width * 0.6` wide.
        let ry = horizon * 0.32;
        let u = (x - self.sun.0 as f64 * self.px) * ry / (self.width * 0.6);
        let d = (u * u + (y - horizon).powi(2)).sqrt() / ry;
        if d < 1. {
            let stops = [
                (0., SKY[14], 0.9),
                (0.3, SKY[13], 0.55),
                (0.65, SKY[12], 0.18),
                (1., SKY[12], 0.),
            ];
            c = over(c, gradient(&stops, d));
        }
        let fog_top = self.fog_top as f64 * self.px;
        if y >= fog_top {
            let t = clamp((y - fog_top) / (horizon - fog_top), 0., 1.);
            c = over(c, (rgb(FOG), 0.3 * t));
        }
        c
    }

    /// The sun's halo at a point, its brightest at `a` (the website breathes it between 0.45 and
    /// 0.5).
    fn halo(&self, x: f64, y: f64, a: f64) -> ([f64; 3], f64) {
        let (sx, sy) = (self.sun.0 as f64 * self.px, self.sun.1 as f64 * self.px);
        let r = self.sun.2 as f64 * 2.4 * self.px;
        let d = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt() / r;
        gradient(
            &[(0., HALO, a), (0.45, HALO, a * 0.3), (1., HALO, 0.)],
            d.min(1.),
        )
    }

    /// The pixel layer's moving parts at `time` (clouds, then the sun), as runs of one colour:
    /// `(x, y, length, colour)` in texels, fog already laid over the ones in its band.
    fn runs(&self, time: f64, view: (f64, f64)) -> Vec<(i32, i32, i32, u32)> {
        let mut runs = Vec::new();
        let push = |runs: &mut Vec<(i32, i32, i32, u32)>, x: i32, y: i32, c: u32| {
            let c = self.fogged(x, y, c);
            match runs.last_mut() {
                Some((rx, ry, len, rc)) if *ry == y && *rx + *len == x && *rc == c => *len += 1,
                _ => runs.push((x, y, 1, c)),
            }
        };
        for c in &self.clouds {
            let span = (self.w + c.w + 16) as f64;
            let shifted = (c.x0 + c.speed * time - view.0 * c.depth * 3.) % span;
            let left = round((shifted + span) % span) as i32 - c.w - 8;
            for cy in 0..c.h {
                let y = c.y + cy;
                if y < 0 || y >= self.h {
                    continue;
                }
                for cx in 0..c.w {
                    let m = c.mask[(cy * c.w + cx) as usize];
                    let x = left + cx;
                    if m == 0 || x < 0 || x >= self.w {
                        continue;
                    }
                    let color = match m {
                        2 => mix(c.body, RIM, c.rim_base + 0.5 * self.glow[x as usize]),
                        3 => mix(c.body, CLOUD_TOP, 0.14),
                        _ => c.body,
                    };
                    push(&mut runs, x, y, color);
                }
            }
        }
        // The sun, in slices that drift down (`drawSun`).
        let (sx, sy, r) = (
            self.sun.0 as f64 + round(-view.0 * 1.5),
            self.sun.1 as f64,
            self.sun.2 as f64,
        );
        let stripe = (r / 5.).round().max(4.);
        let phase = time * 2.2;
        let first = (sy - r).ceil().max(0.) as i32;
        let last = ((sy + r).floor() as i32).min(self.h - 1);
        for y in first..=last {
            let dy = y as f64 + 0.5 - sy;
            let half = (r * r - dy * dy).sqrt();
            if half.is_nan() || half <= 0. {
                continue;
            }
            let v = (dy + r) / (2. * r);
            if v > 0.38 {
                let gap = 1. + (v - 0.38) / (1. - 0.38) * stripe * 0.6;
                if (y as f64 - sy - phase).rem_euclid(stripe) < gap {
                    continue;
                }
            }
            let f = v * (SUN.len() - 1) as f64;
            let i = f.floor() as usize;
            let color = mix(SUN[i], SUN[(i + 1).min(SUN.len() - 1)], f - i as f64);
            let from = round(sx - half).max(0.) as i32;
            let to = (round(sx + half) as i32).min(self.w);
            for x in from..to {
                push(&mut runs, x, y, color);
            }
        }
        runs
    }

    /// `c` at texel (`x`, `y`) with the fog the first ridge lays over everything drawn so far.
    fn fogged(&self, x: i32, y: i32, c: u32) -> u32 {
        if y < self.fog_top.max(0) || y >= self.fog_bottom.min(self.h) {
            return c;
        }
        let t = if y < self.horizon {
            (y - self.fog_top) as f64 / (self.horizon - self.fog_top) as f64
        } else {
            1. - (y - self.horizon) as f64 / (self.fog_bottom - self.horizon) as f64
        };
        let g = self.glow[(x.clamp(0, self.w - 1)) as usize];
        mix(c, FOG, t * t * (0.16 + 0.3 * g))
    }

    /// Ridge `n` as texels (straight RGBA, 0 where empty), all its columns (`w + 2 × MARGIN`)
    /// from its highest texel to the bottom of the window plus `extra` rows, and the first row's
    /// texel `y`.
    fn ridge_texels(&self, n: usize, extra: i32) -> (Vec<[u8; 4]>, i32, i32, i32) {
        let ridge = &self.ridges[n];
        let cols = ridge.top.len() as i32;
        let tallest = ridge
            .trees
            .iter()
            .map(|&(_, s)| TREES[s].len() as i32)
            .max()
            .unwrap_or(0);
        let top_row = ridge.top.iter().copied().min().unwrap_or(0) - tallest;
        let rows = self.h + extra - top_row;
        let mut out = vec![[0u8; 4]; (cols * rows).max(0) as usize];
        let glow = |i: i32| self.glow[(i - MARGIN).clamp(0, self.w - 1) as usize];
        let mut put = |x: i32, y: i32, c: u32| {
            if x >= 0 && x < cols && y >= top_row && y < top_row + rows {
                let [r, g, b] = [16, 8, 0].map(|s| ((c >> s) & 255) as u8);
                out[((y - top_row) * cols + x) as usize] = [r, g, b, 255];
            }
        };
        for i in 0..cols {
            let top = ridge.top[i as usize];
            let left = ridge.top[(i - 1).max(0) as usize];
            let right = ridge.top[(i + 1).min(cols - 1) as usize];
            let rim = mix(ridge.color, ridge.rim, glow(i) * ridge.rim_strength);
            for y in top.max(top_row)..top_row + rows {
                let c = if y == top || y < left || y < right {
                    rim
                } else {
                    ridge.color
                };
                let c = if n == 0 {
                    self.fogged(i - MARGIN, y, c)
                } else {
                    c
                };
                put(i, y, c);
            }
        }
        for &(col, sprite) in &ridge.trees {
            let ground = ridge.top[col as usize];
            let rows_ = TREES[sprite];
            let trunk = rows_[rows_.len() - 1].find('#').unwrap_or(0) as i32;
            for (ty, row) in rows_.iter().enumerate() {
                let y = ground - rows_.len() as i32 + ty as i32;
                for (tx, ch) in row.chars().enumerate() {
                    if ch == '#' {
                        put(col - trunk + tx as i32, y, ridge.color);
                    }
                }
            }
        }
        (out, cols, rows, top_row)
    }
}

fn rgb(c: u32) -> [f64; 3] {
    [16, 8, 0].map(|s| ((c >> s) & 255) as f64 / 255.)
}

fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn over(under: [f64; 3], (c, a): ([f64; 3], f64)) -> [f64; 3] {
    [0, 1, 2].map(|i| c[i] * a + under[i] * (1. - a))
}

/// A canvas gradient's colour and opacity at `d` between its stops (interpolated unpremultiplied).
fn gradient(stops: &[(f64, u32, f64)], d: f64) -> ([f64; 3], f64) {
    let mut i = 0;
    while i + 2 < stops.len() && d > stops[i + 1].0 {
        i += 1;
    }
    let (p0, c0, a0) = stops[i];
    let (p1, c1, a1) = stops[i + 1];
    let t = clamp((d - p0) / (p1 - p0), 0., 1.);
    (lerp(rgb(c0), rgb(c1), t), a0 + (a1 - a0) * t)
}

fn byte(v: f64) -> u8 {
    (clamp(v, 0., 1.) * 255.).round() as u8
}

/// A smooth picture of `w` × `h` texels, stretched over its place.
#[derive(Clone)]
struct Smooth {
    image: Arc<RenderImage>,
    w: usize,
    h: usize,
}

impl Smooth {
    /// The texels' colours and opacities. The image gets a border of one texel, its edges
    /// repeated: GPUI samples its atlas linearly, so a stretched image's outer half-texels would
    /// otherwise blend in whatever image sits next to it in the atlas (a dark seam).
    fn new(w: usize, h: usize, pixels: impl Fn(usize, usize) -> ([f64; 3], f64)) -> Smooth {
        let mut buf = image::RgbaImage::new(w as u32 + 2, h as u32 + 2);
        for (y, row) in buf.rows_mut().enumerate() {
            for (x, p) in row.enumerate() {
                let (c, a) = pixels(x.clamp(1, w) - 1, y.clamp(1, h) - 1);
                // BGRA, straight alpha: GPUI blends images by their alpha.
                *p = image::Rgba([byte(c[2]), byte(c[1]), byte(c[0]), byte(a)]);
            }
        }
        Smooth {
            image: Arc::new(RenderImage::new([image::Frame::new(buf)])),
            w,
            h,
        }
    }

    /// Paints the picture over `bounds`, its border just outside them.
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let tx = f32::from(bounds.size.width) / self.w as f32;
        let ty = f32::from(bounds.size.height) / self.h as f32;
        let outer = Bounds::new(
            bounds.origin - point(px(tx), px(ty)),
            bounds.size + size(px(tx * 2.), px(ty * 2.)),
        );
        let _ = window.paint_image(
            bounds,
            outer,
            Corners::default(),
            self.image.clone(),
            0,
            false,
        );
    }
}

/// One window size's images: the sky, the halo at full strength, and the three ridges with
/// where each one's top row is.
struct Baked {
    key: (u32, u32, u32),
    model: Model,
    sky: Smooth,
    halo: (Smooth, Bounds<Pixels>),
    ridges: Vec<(Arc<RenderImage>, i32, i32, i32)>,
}

/// The rows a ridge image has below the window, for it to rise with the pointer.
const RIDGE_EXTRA: i32 = 4;

impl Baked {
    fn new(width: f64, height: f64, scale: f32) -> Baked {
        let model = Model::new(width, height);
        // The sky's gradients are smooth: half the window's logical size is enough.
        let (sw, sh) = (
            ((width / 2.).ceil() as usize).max(1),
            ((height / 2.).ceil() as usize).max(1),
        );
        let sky = Smooth::new(sw, sh, |x, y| {
            (
                model.sky_rgb((x as f64 + 0.5) * 2., (y as f64 + 0.5) * 2.),
                1.,
            )
        });
        let r = model.sun.2 as f64 * 2.4 * model.px;
        let (cx, cy) = (model.sun.0 as f64 * model.px, model.sun.1 as f64 * model.px);
        let side = ((r * 2.) / 2.).ceil().max(1.) as usize;
        let halo = Smooth::new(side, side, |x, y| {
            model.halo(
                cx - r + (x as f64 + 0.5) * 2.,
                cy - r + (y as f64 + 0.5) * 2.,
                0.5,
            )
        });
        let halo_bounds = Bounds::new(
            point(px((cx - r) as f32), px((cy - r) as f32)),
            size(px((side * 2) as f32), px((side * 2) as f32)),
        );
        let ridges = (0..model.ridges.len())
            .map(|n| {
                let (texels, cols, rows, top) = model.ridge_texels(n, RIDGE_EXTRA);
                let image = art::bake(
                    &texels,
                    cols as usize,
                    rows as usize,
                    model.px as f32,
                    scale,
                );
                (image, cols, rows, top)
            })
            .collect();
        Baked {
            key: key(width, height, scale),
            model,
            sky,
            halo: (halo, halo_bounds),
            ridges,
        }
    }

    fn images(self) -> impl Iterator<Item = Arc<RenderImage>> {
        [self.sky.image, self.halo.0.image]
            .into_iter()
            .chain(self.ridges.into_iter().map(|r| r.0))
    }
}

fn key(width: f64, height: f64, scale: f32) -> (u32, u32, u32) {
    (
        width.round() as u32,
        height.round() as u32,
        (scale * 100.).round() as u32,
    )
}

/// The scene's view: the window's background, drawn again on its own while it moves.
pub struct Scene {
    baked: Option<Baked>,
    replaced: Vec<Arc<RenderImage>>,
    clock: Clock,
    unfocused: Option<Instant>,
    /// The pointer's parallax: where the view is, and where it eases to.
    view: (f64, f64),
    target: (f64, f64),
    last: Option<Instant>,
    /// The frame timer in flight, if any.
    waking: bool,
}

impl Scene {
    pub fn new() -> Scene {
        Scene {
            baked: None,
            replaced: Vec::new(),
            clock: Clock::default(),
            unfocused: None,
            view: (0., 0.),
            target: (0., 0.),
            last: None,
            waking: false,
        }
    }

    /// The pointer moved to `at` in a window `size` big: the ridges, clouds and stars lean
    /// towards it (the website's `onPointer`).
    pub fn point(
        &mut self,
        at: gpui::Point<Pixels>,
        window: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if motion() == Motion::Still {
            return;
        }
        let w = f64::from(f32::from(window.width)).max(1.);
        let h = f64::from(f32::from(window.height)).max(1.);
        self.target = (
            f64::from(f32::from(at.x)) / w * 2. - 1.,
            f64::from(f32::from(at.y)) / h * 2. - 1.,
        );
        cx.notify();
    }

    fn wake(&mut self, after: Duration, cx: &mut Context<Self>) {
        if self.waking {
            return;
        }
        self.waking = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                this.waking = false;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Scene {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.replaced.drain(..) {
            let _ = window.drop_image(image);
        }
        let viewport = window.viewport_size();
        let (width, height) = (
            f64::from(f32::from(viewport.width)),
            f64::from(f32::from(viewport.height)),
        );
        let scale = window.scale_factor();
        if self.baked.as_ref().map(|b| b.key) != Some(key(width, height, scale)) {
            let baked = Baked::new(width, height, scale);
            crate::glass::set_backdrop(&baked.model);
            if let Some(old) = self.baked.replace(baked) {
                self.replaced.extend(old.images());
            }
        }

        let now = Instant::now();
        if window.is_window_active() {
            self.unfocused = None;
        } else {
            self.unfocused.get_or_insert(now);
        }
        let run = match motion() {
            Motion::Still => false,
            Motion::InFront => self.unfocused.is_none_or(|since| now - since < UNFOCUSED),
            Motion::Always => true,
        };
        self.clock.set(run, now);
        // The view eases towards the pointer as the website's does (`dt * 3` a frame).
        let dt = self.last.map_or(0., |l| (now - l).as_secs_f64().min(0.1));
        self.last = run.then_some(now);
        let ease = (dt * 3.).min(1.);
        self.view.0 += (self.target.0 - self.view.0) * ease;
        self.view.1 += (self.target.1 - self.view.1) * ease;
        let time = self.clock.at(now).as_secs_f64();
        if run {
            self.wake(TICK, cx);
        }

        let baked = self.baked.as_ref().expect("baked above");
        let m = &baked.model;
        let view = self.view;
        let tx = m.px as f32;
        let halo_strength = (0.9 + 0.1 * (time * 0.9).sin()) as f32;

        // The sky's moving parts (`renderSky`): stars and the odd meteor, in logical pixels.
        let sky_height = m.horizon as f64 * m.px;
        let shift = -view.0 * 4.;
        let mut stars: Vec<(Bounds<Pixels>, f32, Hsla)> = Vec::new();
        for s in &m.stars {
            let twinkle = 0.5 + 0.5 * (time * s.speed + s.phase).sin();
            let alpha =
                s.b * (1. - smoothstep(0.4, 0.82, s.y / sky_height)) * (0.35 + 0.65 * twinkle);
            if alpha < 0.04 {
                continue;
            }
            let x = s.x + shift;
            let color = |a: f64| -> Hsla {
                let c = if s.cool { STAR_COOL } else { STAR_WARM };
                gpui::rgba(c << 8 | (a * 255.).round() as u32).into()
            };
            let r = s.size / 2.;
            stars.push((rect(x - r, s.y - r, s.size, s.size), r as f32, color(alpha)));
            if s.sparkle && twinkle > 0.65 {
                let arm = (twinkle - 0.65) * 10.;
                stars.push((
                    rect(x - arm, s.y - 0.3, arm * 2., 0.6),
                    0.,
                    color(alpha * 0.45),
                ));
                stars.push((
                    rect(x - 0.3, s.y - arm, 0.6, arm * 2.),
                    0.,
                    color(alpha * 0.45),
                ));
            }
        }
        stars.extend(meteor(time, m.width, sky_height));

        let runs = m.runs(time, view);
        let flies: Vec<_> = m
            .flies
            .iter()
            .filter_map(|f| {
                let a = (time * f.blink + f.phase).sin().max(0.).powi(3);
                if a < 0.05 {
                    return None;
                }
                let x = round(f.x + (time * 0.35 + f.phase).sin() * 5. - view.0 * 8.) as i32;
                let y = round(f.y + (time * 0.5 + f.phase * 1.7).sin() * 2.5) as i32;
                (x >= 1 && x < m.w - 1 && y >= 1 && y < m.h - 1).then_some((x, y, a))
            })
            .collect();

        let mut root = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .bg(gpui::rgb(horizon_theme::glass::PAGE))
            .child(layer(baked.sky.clone(), None))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .opacity(halo_strength)
                    .child(layer(baked.halo.0.clone(), Some(baked.halo.1))),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        for (b, r, c) in stars {
                            let b = Bounds::new(b.origin + bounds.origin, b.size);
                            window.paint_quad(fill(b, c).corner_radii(px(r)));
                        }
                        for (x, y, len, c) in runs {
                            let b = Bounds::new(
                                bounds.origin + point(px(x as f32 * tx), px(y as f32 * tx)),
                                size(px(len as f32 * tx), px(tx)),
                            );
                            window.paint_quad(fill(b, gpui::rgb(c)));
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        for (n, (image, cols, rows, top)) in baked.ridges.iter().enumerate() {
            let depth = m.ridges[n].depth;
            let ox = round(-view.0 * depth * 8.) as f32;
            let oy = round(-view.1 * depth * 2.) as f32;
            root = root.child(
                img(image.clone())
                    .absolute()
                    .left(px((ox - MARGIN as f32) * tx))
                    .top(px((*top as f32 + oy) * tx))
                    .w(px(*cols as f32 * tx))
                    .h(px(*rows as f32 * tx)),
            );
        }
        root.child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    for (x, y, a) in flies {
                        let at = |dx: i32, dy: i32, a: f64| {
                            let b = Bounds::new(
                                bounds.origin
                                    + point(px((x + dx) as f32 * tx), px((y + dy) as f32 * tx)),
                                size(px(tx), px(tx)),
                            );
                            fill(b, alpha(FIREFLY, a))
                        };
                        window.paint_quad(at(0, 0, a));
                        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                            window.paint_quad(at(dx, dy, a * 0.25));
                        }
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}

/// A layer of the scene painting `picture` over the whole window, or over `at` (window
/// coordinates).
fn layer(picture: Smooth, at: Option<Bounds<Pixels>>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let place = at.map_or(bounds, |b| Bounds::new(bounds.origin + b.origin, b.size));
            picture.paint(place, window);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Bounds<Pixels> {
    Bounds::new(
        point(px(x as f32), px(y as f32)),
        size(px(w as f32), px(h as f32)),
    )
}

fn alpha(c: u32, a: f64) -> Rgba {
    gpui::rgba(c << 8 | (clamp(a, 0., 1.) * 255.).round() as u32)
}

/// The meteor of the 9-second window `time` falls in, if one is crossing the sky: its trail as
/// small dots fading from the head (the website strokes a 1.5 px line with a gradient).
fn meteor(time: f64, width: f64, sky_height: f64) -> Vec<(Bounds<Pixels>, f32, Hsla)> {
    let window9 = (time / 9.).floor();
    let mut rand = Rng((window9 as i64 * 7919 + 13) as i32);
    let age = time - window9 * 9. - (1. + rand.next() * 5.);
    if !(age > 0. && age < 0.9 && rand.next() < 0.8) {
        return Vec::new();
    }
    let dir = if rand.next() < 0.5 { -1. } else { 1. };
    let travel = width * 0.5 * age;
    let hx = width * (0.15 + rand.next() * 0.7) + dir * 0.87 * travel;
    let hy = sky_height * (0.05 + rand.next() * 0.22) + 0.5 * travel;
    let life = (age / 0.9 * PI).sin();
    if hy >= sky_height * 0.7 {
        return Vec::new();
    }
    let steps = 74;
    (0..steps)
        .map(|i| {
            let f = i as f64 / steps as f64;
            let (x, y) = (hx - dir * 0.87 * 110. * f, hy - 0.5 * 110. * f);
            let c: Hsla = alpha(STAR_WARM, life * (1. - f)).into();
            (rect(x - 0.75, y - 0.75, 1.5, 1.5), 0.75, c)
        })
        .collect()
}

/// The scene blurred as `.hzl-glass` and `.hzl-float` see it through their backdrop filters
/// (blur, then `saturate(140%)`, and `brightness(0.62)` for the float): one colour per texel at
/// the scene's first frame, without the stars (a 24 px blur leaves nothing of them).
pub struct Backdrop {
    pub w: usize,
    pub h: usize,
    /// Logical pixels per texel.
    pub px: f64,
    pub glass: Vec<[f32; 3]>,
    pub float: Vec<[f32; 3]>,
}

impl Backdrop {
    pub fn new(m: &Model, blur: f64) -> Backdrop {
        let (w, h) = (m.w as usize, m.h as usize);
        let mut base = vec![[0f64; 3]; w * h];
        for (i, c) in base.iter_mut().enumerate() {
            let (x, y) = (((i % w) as f64 + 0.5) * m.px, ((i / w) as f64 + 0.5) * m.px);
            *c = over(m.sky_rgb(x, y), m.halo(x, y, 0.45));
        }
        let mut put = |x: i32, y: i32, c: u32| {
            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                base[y as usize * w + x as usize] = rgb(c);
            }
        };
        for (x, y, len, c) in m.runs(0., (0., 0.)) {
            for dx in 0..len {
                put(x + dx, y, c);
            }
        }
        for n in 0..m.ridges.len() {
            let (texels, cols, rows, top) = m.ridge_texels(n, 0);
            for row in 0..rows {
                for col in 0..cols {
                    let [r, g, b, a] = texels[(row * cols + col) as usize];
                    if a > 0 {
                        put(
                            col - MARGIN,
                            top + row,
                            (r as u32) << 16 | (g as u32) << 8 | b as u32,
                        );
                    }
                }
            }
        }
        let filtered = |sigma: f64, brightness: f64| -> Vec<[f32; 3]> {
            blur_rgb(&base, w, h, sigma / m.px)
                .into_iter()
                .map(|c| {
                    let s = saturate(c, 1.4);
                    s.map(|v| (clamp(v, 0., 1.) * brightness) as f32)
                })
                .collect()
        };
        Backdrop {
            w,
            h,
            px: m.px,
            glass: filtered(blur, 1.),
            float: filtered(blur + 24., 0.62),
        }
    }

    /// The blurred colour at a point in logical pixels (bilinear between texel centres).
    pub fn at(&self, float: bool, x: f64, y: f64) -> [f32; 3] {
        let grid = if float { &self.float } else { &self.glass };
        let fx = clamp(x / self.px - 0.5, 0., (self.w - 1) as f64);
        let fy = clamp(y / self.px - 0.5, 0., (self.h - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let px = |x: usize, y: usize| grid[y * self.w + x];
        let l = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
        l(
            l(px(x0, y0), px(x1, y0), tx),
            l(px(x0, y1), px(x1, y1), tx),
            ty,
        )
    }
}

/// A Gaussian blur of standard deviation `sigma` texels, its edges held (CSS blurs a backdrop
/// with its edge pixels repeated).
fn blur_rgb(src: &[[f64; 3]], w: usize, h: usize, sigma: f64) -> Vec<[f64; 3]> {
    if sigma <= 0. {
        return src.to_vec();
    }
    let radius = (sigma * 3.).ceil() as i64;
    let kernel: Vec<f64> = (-radius..=radius)
        .map(|i| (-(i * i) as f64 / (2. * sigma * sigma)).exp())
        .collect();
    let sum: f64 = kernel.iter().sum();
    let pass = |src: &[[f64; 3]], horizontal: bool| -> Vec<[f64; 3]> {
        let mut out = vec![[0.; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.; 3];
                for (k, wgt) in kernel.iter().enumerate() {
                    let d = k as i64 - radius;
                    let (sx, sy) = if horizontal {
                        ((x as i64 + d).clamp(0, w as i64 - 1) as usize, y)
                    } else {
                        (x, (y as i64 + d).clamp(0, h as i64 - 1) as usize)
                    };
                    let c = src[sy * w + sx];
                    for i in 0..3 {
                        acc[i] += c[i] * wgt;
                    }
                }
                out[y * w + x] = acc.map(|v| v / sum);
            }
        }
        out
    };
    pass(&pass(src, true), false)
}

/// CSS `saturate(s)` (Filter Effects' matrix).
fn saturate([r, g, b]: [f64; 3], s: f64) -> [f64; 3] {
    [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The website's own code (horizon-scene.tsx's `rng`, `buildScene`, `renderPixels`, run in
    /// Node at 1280 × 772) gave these: the first draws of seed 7, the scene's measures, each cloud's
    /// row, size and mask sum, and an FNV-1a hash of its whole pixel buffer at 0 s and 5.3 s.
    #[test]
    fn the_scene_matches_the_website_pixel_for_pixel() {
        let mut r = Rng(7);
        let draws: Vec<f64> = (0..3).map(|_| r.next()).collect();
        assert_eq!(
            draws,
            [0.011704753153026104, 0.06195825757458806, 0.97690763277933]
        );
        let m = Model::new(1280., 772.);
        assert_eq!(
            (m.px, m.w, m.h, m.horizon, m.sun),
            (4., 320, 193, 161, (230, 147, 25))
        );
        assert_eq!(m.stars.len(), 159);
        assert_eq!(
            (m.stars[0].x, m.stars[0].y),
            (14.982084035873413, 5.233893274599216)
        );
        let clouds: Vec<_> = m
            .clouds
            .iter()
            .map(|c| (c.y, c.w, c.h, c.mask.iter().map(|&v| v as u32).sum::<u32>()))
            .collect();
        assert_eq!(
            clouds,
            [
                (134, 60, 15, 437),
                (126, 39, 9, 250),
                (124, 32, 7, 202),
                (133, 42, 3, 245),
                (144, 48, 6, 353),
                (138, 54, 4, 270),
                (151, 61, 4, 308),
                (139, 36, 6, 225)
            ]
        );
        assert_eq!(fnv(&composite(&m, 0.)), 0xa8f0b1c50b378d7a);
        assert_eq!(fnv(&composite(&m, 5.3)), 0x6c05b4e8bdbbd1fc);
    }

    /// The pixel layer as the website's buffer holds it (`-1` empty), built from what this module
    /// draws: the runs, the ridges' images, and the fireflies mixed in as `renderPixels` does.
    fn composite(m: &Model, time: f64) -> Vec<i64> {
        let (w, h) = (m.w as usize, m.h as usize);
        let mut buf = vec![-1i64; w * h];
        for (x, y, len, c) in m.runs(time, (0., 0.)) {
            for dx in 0..len {
                buf[y as usize * w + (x + dx) as usize] = c as i64;
            }
        }
        for n in 0..m.ridges.len() {
            let (texels, cols, rows, top) = m.ridge_texels(n, 0);
            for row in 0..rows {
                for col in MARGIN..MARGIN + m.w {
                    let [r, g, b, a] = texels[(row * cols + col) as usize];
                    let y = top + row;
                    if a > 0 && y >= 0 && y < m.h {
                        buf[y as usize * w + (col - MARGIN) as usize] =
                            ((r as i64) << 16) | ((g as i64) << 8) | b as i64;
                    }
                }
            }
        }
        for f in &m.flies {
            let a = (time * f.blink + f.phase).sin().max(0.).powi(3);
            if a < 0.05 {
                continue;
            }
            let x = round(f.x + (time * 0.35 + f.phase).sin() * 5.) as i32;
            let y = round(f.y + (time * 0.5 + f.phase * 1.7).sin() * 2.5) as i32;
            if x < 1 || x >= m.w - 1 || y < 1 || y >= m.h - 1 {
                continue;
            }
            let i = y as usize * w + x as usize;
            if buf[i] < 0 {
                continue;
            }
            buf[i] = mix(buf[i] as u32, FIREFLY, a) as i64;
            for j in [i - 1, i + 1, i - w, i + w] {
                if buf[j] >= 0 {
                    buf[j] = mix(buf[j] as u32, FIREFLY, a * 0.25) as i64;
                }
            }
        }
        buf
    }

    fn fnv(values: &[i64]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for &v in values {
            let mut x = (v + 1) as u32;
            for _ in 0..4 {
                h ^= (x & 0xff) as u64;
                h = h.wrapping_mul(0x100000001b3);
                x >>= 8;
            }
        }
        h
    }

    #[test]
    fn mix_cuts_channels_like_javascript() {
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x7f7f7f);
        assert_eq!(mix(0x102030, 0x102030, 0.3), 0x102030);
    }

    #[test]
    fn the_land_sits_at_the_bottom_with_the_sun_right_of_centre() {
        let m = Model::new(1280., 772.);
        assert_eq!(m.px, 4.);
        assert!(m.sun.0 > m.w / 2);
        assert!(m.horizon > m.h / 2 && m.horizon < m.h);
        // Every column of the nearest ridge reaches the bottom.
        let (texels, cols, rows, _) = m.ridge_texels(2, 0);
        assert!((0..cols).all(|c| texels[((rows - 1) * cols + c) as usize][3] == 255));
    }
}
