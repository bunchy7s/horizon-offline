//! `MipmapGenerator`: the mip levels of a sprite. Cutout textures keep the
//! alpha-test coverage of their base level at every level
//! (`scaleAlphaToCoverage`), so thin cutout edges (cactus sides, leaves,
//! flowers) don't erode into gaps with distance; colours average in linear
//! light (`ARGB.meanLinear`).

/// `MipmapStrategy`, from the texture's `.mcmeta` `texture` section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    #[default]
    Auto,
    Mean,
    Cutout,
    StrictCutout,
    DarkCutout,
}
impl Strategy {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "auto" => Self::Auto,
            "mean" => Self::Mean,
            "cutout" => Self::Cutout,
            "strict_cutout" => Self::StrictCutout,
            "dark_cutout" => Self::DarkCutout,
            _ => return None,
        })
    }
}

/// How a sprite's mips are made: its `.mcmeta` `texture` section, whether
/// it has a fully transparent pixel and whether it is an `item/` sprite.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings {
    /// Original square frame edge, before fitting the sprite into the array.
    /// Zero means the supplied image is already at its native size.
    pub native_size: u32,
    pub strategy: Strategy,
    pub alpha_cutoff_bias: f32,
    pub has_transparent: bool,
    pub item: bool,
}

/// A square or rectangular RGBA8 image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}
impl Image {
    fn resized(&self, size: u32) -> Self {
        Self {
            width: size,
            height: size,
            pixels: (0..size * size)
                .map(|i| self.get(i % size * self.width / size, i / size * self.height / size))
                .collect(),
        }
    }
    fn get(&self, x: u32, y: u32) -> [u8; 4] {
        self.pixels[(x + y * self.width) as usize]
    }
    fn set(&mut self, x: u32, y: u32, p: [u8; 4]) {
        self.pixels[(x + y * self.width) as usize] = p;
    }
}

/// `ARGB.SRGB_TO_LINEAR`: an sRGB channel in linear light, 0 to 1023.
fn srgb_to_linear() -> &'static [u16; 256] {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let x = i as f32 / 255.0;
            let linear = if x >= 0.04045 {
                ((f64::from(x) + 0.055) / 1.055).powf(2.4) as f32
            } else {
                x / 12.92
            };
            java_round(linear * 1023.0) as u16
        })
    })
}
/// `ARGB.LINEAR_TO_SRGB`: 1024 linear steps back to sRGB.
fn linear_to_srgb() -> &'static [u8; 1024] {
    static TABLE: std::sync::OnceLock<[u8; 1024]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let x = i as f32 / 1023.0;
            let srgb = if x >= 0.003_130_8 {
                (1.055 * f64::from(x).powf(0.416_666_666_666_666_7) - 0.055) as f32
            } else {
                12.92 * x
            };
            java_round(srgb * 255.0) as u8
        })
    })
}
/// `Math.round(float)`, bit for bit.
fn java_round(a: f32) -> i32 {
    let bits = a.to_bits() as i32;
    let biased_exp = (bits & 0x7F80_0000) >> 23;
    let shift = (22 + 127) - biased_exp;
    if shift & -32 == 0 {
        let mut r = bits & 0x007F_FFFF | 0x0080_0000;
        if bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        a as i32
    }
}
/// `ARGB.alphaFloat`.
fn alpha_float(p: [u8; 4]) -> f32 {
    f32::from(p[3]) / 255.0
}

/// `MipmapGenerator.alphaTestCoverage`: the share of the image (bilinearly
/// subsampled 4x4 per texel) whose scaled alpha passes `alpha_ref`.
fn alpha_test_coverage(image: &Image, alpha_ref: f32, alpha_scale: f32) -> f32 {
    let (width, height) = (image.width as i32, image.height as i32);
    let mut coverage = 0.0f32;
    let a =
        |x: i32, y: i32| (alpha_float(image.get(x as u32, y as u32)) * alpha_scale).clamp(0.0, 1.0);
    for y in 0..height - 1 {
        for x in 0..width - 1 {
            let (a00, a10, a01, a11) = (a(x, y), a(x + 1, y), a(x, y + 1), a(x + 1, y + 1));
            let mut texel = 0.0f32;
            for sy in 0..4 {
                let fy = (sy as f32 + 0.5) / 4.0;
                for sx in 0..4 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let alpha = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if alpha > alpha_ref {
                        texel += 1.0;
                    }
                }
            }
            coverage += texel / 16.0;
        }
    }
    coverage / ((width - 1) * (height - 1)) as f32
}

/// `MipmapGenerator.scaleAlphaToCoverage`: five bisection steps for the
/// alpha scale whose coverage is nearest `desired`, then every alpha scaled
/// and biased.
fn scale_alpha_to_coverage(image: &mut Image, desired: f32, alpha_ref: f32, bias: f32) {
    let (mut min_scale, mut max_scale) = (0.0f32, 4.0f32);
    let mut scale = 1.0f32;
    let mut best_scale = 1.0f32;
    let mut best_error = f32::MAX;
    for _ in 0..5 {
        let current = alpha_test_coverage(image, alpha_ref, scale);
        let error = (current - desired).abs();
        if error < best_error {
            best_error = error;
            best_scale = scale;
        }
        if current < desired {
            min_scale = scale;
        } else if current > desired {
            max_scale = scale;
        } else {
            break;
        }
        scale = (min_scale + max_scale) * 0.5;
    }
    for p in &mut image.pixels {
        let alpha = (alpha_float(*p) * best_scale + bias + 0.025).clamp(0.0, 1.0);
        p[3] = (alpha * 255.0).floor() as u8;
    }
}

/// `TextureUtil.solidify`: every transparent pixel takes the colour of the
/// nearest opaque one (breadth first, keeping its zero alpha).
fn solidify(image: &mut Image) {
    let (width, height) = (image.width as i32, image.height as i32);
    let n = image.pixels.len();
    let mut nearest = vec![[0u8; 4]; n];
    let mut distance = vec![i32::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    for x in 0..width {
        for y in 0..height {
            let i = (x + y * width) as usize;
            let p = image.pixels[i];
            if p[3] != 0 {
                distance[i] = 0;
                nearest[i] = p;
                queue.push_back(i);
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i as i32 % width, i as i32 / width);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && nx < width && ny < height {
                let j = (nx + ny * width) as usize;
                if distance[j] > distance[i] + 1 {
                    distance[j] = distance[i] + 1;
                    nearest[j] = nearest[i];
                    queue.push_back(j);
                }
            }
        }
    }
    for (i, p) in image.pixels.iter_mut().enumerate() {
        if p[3] == 0 {
            let c = nearest[i];
            *p = [c[0], c[1], c[2], 0];
        }
    }
}

/// `TextureUtil.fillEmptyAreasWithDarkColor`: transparent pixels take three
/// quarters of the darkest visible colour.
fn fill_empty_dark(image: &mut Image) {
    let mut darkest = [255u8; 4];
    let mut min = i32::MAX;
    for x in 0..image.width {
        for y in 0..image.height {
            let p = image.get(x, y);
            if p[3] != 0 {
                let brightness = i32::from(p[0]) + i32::from(p[1]) + i32::from(p[2]);
                if brightness < min {
                    min = brightness;
                    darkest = p;
                }
            }
        }
    }
    let dark = |c: u8| (3 * i32::from(c) / 4) as u8;
    let fill = [dark(darkest[0]), dark(darkest[1]), dark(darkest[2]), 0];
    for p in &mut image.pixels {
        if p[3] == 0 {
            *p = fill;
        }
    }
}

/// `ARGB.meanLinear`.
fn mean_linear(c: [[u8; 4]; 4]) -> [u8; 4] {
    let (to_linear, to_srgb) = (srgb_to_linear(), linear_to_srgb());
    let channel = |k: usize| {
        let sum: u32 = c.iter().map(|p| u32::from(to_linear[p[k] as usize])).sum();
        to_srgb[(sum / 4) as usize]
    };
    let alpha = c.iter().map(|p| u32::from(p[3])).sum::<u32>() / 4;
    [channel(0), channel(1), channel(2), alpha as u8]
}

/// `MipmapGenerator.darkenedAlphaBlend`: the visible pixels' mean in linear
/// light, transparent ones counting as black.
fn darkened_alpha_blend(c: [[u8; 4]; 4]) -> [u8; 4] {
    let to_linear = srgb_to_linear();
    let linear = |v: u8| f32::from(to_linear[v as usize]) / 1023.0;
    let mut total = [0.0f32; 4];
    for p in c.iter().filter(|p| p[3] != 0) {
        total[3] += linear(p[3]);
        for k in 0..3 {
            total[k] += linear(p[k]);
        }
    }
    let to_srgb = |v: f32| linear_to_srgb()[(v / 4.0 * 1023.0).floor() as usize];
    [
        to_srgb(total[0]),
        to_srgb(total[1]),
        to_srgb(total[2]),
        to_srgb(total[3]),
    ]
}

/// `MipmapGenerator.generateMipLevels` from the base image (`levels` mips
/// below it). Array-only enlargement must not run extra alpha-coverage steps
/// before the sprite's native mip chain.
pub fn generate(base: Image, levels: u32, settings: Settings) -> Vec<Image> {
    let native = settings.native_size;
    if native.is_power_of_two()
        && native < base.width
        && base.width == base.height
        && base.width.is_power_of_two()
    {
        let extra = (base.width / native).ilog2();
        let mut mips =
            generate_native(base.resized(native), levels.saturating_sub(extra), settings);
        let mut out: Vec<_> = (0..extra.min(levels + 1))
            .map(|level| mips[0].resized(base.width >> level))
            .collect();
        if levels >= extra {
            out.append(&mut mips);
        }
        return out;
    }
    generate_native(base, levels, settings)
}

fn generate_native(mut base: Image, levels: u32, settings: Settings) -> Vec<Image> {
    let Settings {
        strategy,
        alpha_cutoff_bias,
        has_transparent,
        item,
        ..
    } = settings;
    let strategy = match strategy {
        Strategy::Auto if has_transparent => Strategy::Cutout,
        Strategy::Auto => Strategy::Mean,
        s => s,
    };
    if !item {
        match strategy {
            Strategy::Cutout | Strategy::StrictCutout => solidify(&mut base),
            Strategy::DarkCutout => fill_empty_dark(&mut base),
            _ => {}
        }
    }
    let cutout = matches!(
        strategy,
        Strategy::Cutout | Strategy::StrictCutout | Strategy::DarkCutout
    );
    let alpha_ref = if strategy == Strategy::StrictCutout {
        0.3
    } else {
        0.5
    };
    let coverage = if cutout {
        alpha_test_coverage(&base, alpha_ref, 1.0)
    } else {
        0.0
    };
    let mut out = vec![base];
    for _ in 1..=levels {
        let last = out.last().expect("the base level");
        let (width, height) = (last.width >> 1, last.height >> 1);
        let mut data = Image {
            width,
            height,
            pixels: vec![[0; 4]; (width * height) as usize],
        };
        for x in 0..width {
            for y in 0..height {
                let c = [
                    last.get(x * 2, y * 2),
                    last.get(x * 2 + 1, y * 2),
                    last.get(x * 2, y * 2 + 1),
                    last.get(x * 2 + 1, y * 2 + 1),
                ];
                let p = if strategy == Strategy::DarkCutout {
                    darkened_alpha_blend(c)
                } else {
                    mean_linear(c)
                };
                data.set(x, y, p);
            }
        }
        if cutout {
            scale_alpha_to_coverage(&mut data, coverage, alpha_ref, alpha_cutoff_bias);
        }
        out.push(data);
    }
    out
}
