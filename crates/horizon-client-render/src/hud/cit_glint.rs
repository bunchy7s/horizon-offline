//! OptiFine CIT enchantment glints on GUI items (`type=enchantment`, see `crate::cit_glint` for
//! which layers a stack draws): every rule's texture in one texture array, each layer an extra
//! pass over the item's quads with its rule's blend. A layer's vertices carry its texture slot
//! (red) and its intensity (alpha) in their colour; its scrolled UV is worked out on the CPU, as
//! vanilla's glint's is. Without enchantment rules nothing here is made.

use super::HudVertex;
use horizon_optifine::cit::{Blend, Factor, Kind, Rule};
use std::path::Path;

/// Slots the shader's table holds; rules past it draw no layer (and match no stack).
pub const MAX_SLOTS: usize = 256;
/// The largest side the shared texture size takes: bigger glints are scaled down to it.
const MAX_SIDE: u32 = 1024;
/// The most the texture array takes, in bytes: past it the shared size is halved.
const MAX_BYTES: u64 = 64 << 20;

/// One slot's constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// The rule's `r`, `g`, `b`, `a`.
    pub rgba: [f32; 4],
    /// Linear filtering (`blur`), else nearest.
    pub blur: bool,
    /// An index into `GlintTextures::blends`.
    pub blend: u8,
}

/// The enchantment rules' textures, resized to one size.
#[derive(Debug, Default)]
pub struct GlintTextures {
    pub width: u32,
    pub height: u32,
    /// One `width` x `height` RGBA image per slot, slot after slot. Kept after the upload: every
    /// split-screen player's renderer builds its own array from it.
    pub pixels: Vec<u8>,
    /// Per rule (an index into `CitRules::rules`): its slot; `None` for other kinds of rules,
    /// textures that did not load and rules past `MAX_SLOTS`.
    pub slot_of: Vec<Option<u8>>,
    pub slots: Vec<Slot>,
    /// The blends the slots use, once each: the HUD makes a pipeline per entry.
    pub blends: Vec<Option<Blend>>,
}

impl GlintTextures {
    /// The textures of `rules` under the overlay resource root.
    pub fn load(root: &Path, rules: &[Rule]) -> Self {
        let mut images = Vec::new();
        let mut past = 0;
        for (i, rule) in rules.iter().enumerate() {
            let Some(path) = rule.texture.as_deref() else {
                continue;
            };
            if rule.kind != Kind::Enchantment {
                continue;
            }
            if images.len() == MAX_SLOTS {
                past += 1;
                continue;
            }
            match super::decode(&root.join(path)) {
                Ok(image) => images.push((i, image)),
                Err(e) => log::warn!("custom item textures: glint {path} left out: {e}"),
            }
        }
        if past > 0 {
            log::warn!(
                "custom item textures: {past} glint rules past the first {MAX_SLOTS} left out"
            );
        }
        Self::new(rules, images, MAX_BYTES)
    }

    /// `images`: (rule, (width, height, RGBA)), in at most `budget` bytes.
    fn new(rules: &[Rule], images: Vec<(usize, (u32, u32, Vec<u8>))>, budget: u64) -> Self {
        let sizes = images.iter().map(|(_, (w, h, _))| (*w, *h));
        let Some((width, height)) = shared_size(sizes, images.len(), budget) else {
            return Self::default();
        };
        let mut out = Self {
            width,
            height,
            slot_of: vec![None; rules.len()],
            ..Self::default()
        };
        for (rule, (w, h, rgba)) in images {
            let glint = &rules[rule].glint;
            out.pixels
                .extend(resize(&rgba, (w, h), (width, height), glint.blur));
            let blend = match out.blends.iter().position(|b| *b == glint.blend) {
                Some(i) => i,
                None => {
                    out.blends.push(glint.blend);
                    out.blends.len() - 1
                }
            };
            out.slot_of[rule] = Some(out.slots.len() as u8);
            out.slots.push(Slot {
                rgba: glint.rgba,
                blur: glint.blur,
                blend: blend as u8,
            });
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// The size every slot takes: the largest width and height of `sizes`, each at most
/// `MAX_SIDE`, then halved until `layers` RGBA images of it fit in `budget` bytes.
fn shared_size(
    sizes: impl Iterator<Item = (u32, u32)>,
    layers: usize,
    budget: u64,
) -> Option<(u32, u32)> {
    let (mut w, mut h) = sizes.fold(None, |max: Option<(u32, u32)>, (w, h)| {
        let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
        Some(max.map_or((w, h), |(mw, mh)| (mw.max(w), mh.max(h))))
    })?;
    let bytes = |w: u32, h: u32| u64::from(w) * u64::from(h) * 4 * layers as u64;
    while bytes(w, h) > budget && (w > 1 || h > 1) {
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
    Some((w, h))
}

/// `rgba` (`from` texels) scaled to `to`: bilinearly for a `blur` glint, as the GPU would have
/// filtered the texture at its own size, else to the nearest texel. Both wrap at the edges, as
/// the glint repeats over the item.
fn resize(rgba: &[u8], from: (u32, u32), to: (u32, u32), smooth: bool) -> Vec<u8> {
    if from == to {
        return rgba.to_vec();
    }
    let ((w, h), (to_w, to_h)) = (from, to);
    let texel = |x: i64, y: i64| {
        let (x, y) = (x.rem_euclid(i64::from(w)), y.rem_euclid(i64::from(h)));
        let at = (y as usize * w as usize + x as usize) * 4;
        &rgba[at..at + 4]
    };
    // Where texel `i` of `n` samples the source's `size` texels, in source texels.
    let centre = |i: u32, n: u32, size: u32| (f64::from(i) + 0.5) * f64::from(size) / f64::from(n);
    let mut out = Vec::with_capacity(to_w as usize * to_h as usize * 4);
    for y in 0..to_h {
        let sy = centre(y, to_h, h);
        for x in 0..to_w {
            let sx = centre(x, to_w, w);
            if !smooth {
                let pick = |i: u32, n: u32, size: u32| {
                    (u64::from(i) * u64::from(size) / u64::from(n)) as i64
                };
                out.extend_from_slice(texel(pick(x, to_w, w), pick(y, to_h, h)));
                continue;
            }
            let (fx, fy) = (sx - 0.5, sy - 0.5);
            let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
            let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
            let [a, b, c, d] = [
                texel(x0, y0),
                texel(x0 + 1, y0),
                texel(x0, y0 + 1),
                texel(x0 + 1, y0 + 1),
            ];
            let lerp =
                |p: &[u8], q: &[u8], k: usize| f64::from(p[k]) * (1.0 - tx) + f64::from(q[k]) * tx;
            out.extend((0..4).map(|k| {
                let (top, bottom) = (lerp(a, b, k), lerp(c, d, k));
                (top * (1.0 - ty) + bottom * ty).round() as u8
            }));
        }
    }
    out
}

/// CIT Resewn's glint texturing for one layer: `translation(-x, y)`, `rotateZ(rotation)`, then
/// `scale`, where `x` and `y` run through vanilla's 110000 and 30000 periods of the clock
/// `millis * speed` (vanilla's own glint runs at `millis * glintSpeed * 8`). `rotation` is in
/// degrees, as OptiFine's doc has it.
pub fn layer_uv(model: [f32; 2], millis: f32, speed: f32, rotation: f32, scale: f32) -> [f32; 2] {
    let clock = f64::from(millis) * f64::from(speed);
    let x = (clock % 110_000.0 / 110_000.0) as f32;
    let y = (clock % 30_000.0 / 30_000.0) as f32;
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (u, v) = (model[0] * scale, model[1] * scale);
    [u * cos - v * sin - x, u * sin + v * cos + y]
}

/// The colour of a layer's vertices: its slot in red, its intensity in alpha.
pub fn vertex_color(slot: u8, intensity: f32) -> u32 {
    let alpha = (intensity.clamp(0.0, 1.0) * 255.0).round() as u8;
    super::rgba(slot, 0, 0, alpha)
}

fn factor(f: Factor) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as W;
    match f {
        Factor::Zero => W::Zero,
        Factor::One => W::One,
        Factor::SrcColor => W::Src,
        Factor::OneMinusSrcColor => W::OneMinusSrc,
        Factor::SrcAlpha => W::SrcAlpha,
        Factor::OneMinusSrcAlpha => W::OneMinusSrcAlpha,
        Factor::DstAlpha => W::DstAlpha,
        Factor::OneMinusDstAlpha => W::OneMinusDstAlpha,
        Factor::DstColor => W::Dst,
        Factor::OneMinusDstColor => W::OneMinusDst,
        Factor::SrcAlphaSaturate => W::SrcAlphaSaturated,
    }
}

/// `glBlendFuncSeparate` with the rule's factors; `replace` draws unblended.
pub fn blend_state(blend: Option<Blend>) -> wgpu::BlendState {
    let Some(b) = blend else {
        return wgpu::BlendState::REPLACE;
    };
    let component = |src, dst| wgpu::BlendComponent {
        src_factor: factor(src),
        dst_factor: factor(dst),
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component(b.src, b.dst),
        alpha: component(b.src_alpha, b.dst_alpha),
    }
}

/// The GPU side: the texture array and slot table (bind group 1 of `fs_cit_glint`), and one
/// pipeline per blend.
pub(super) struct CitGlintPass {
    group: wgpu::BindGroup,
    pipelines: Vec<wgpu::RenderPipeline>,
}

impl CitGlintPass {
    /// `None` without textures. `hud` is the HUD's bind group layout (group 0).
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        hud: &wgpu::BindGroupLayout,
        textures: &GlintTextures,
    ) -> Option<Self> {
        use wgpu::util::DeviceExt;
        if textures.is_empty() {
            return None;
        }
        // wgpu's OpenGL backend makes 1 layer a 2D texture and 6n a cube map
        // (`gfx::array_layers`): pad with empty layers, within the GPU's limit. Slots past the
        // layers kept sample the last one.
        let gl = device.adapter_info().backend == wgpu::Backend::Gl;
        let max = device.limits().max_texture_array_layers.max(1);
        let slots = textures.slots.len() as u32;
        let mut kept = slots.min(max);
        while kept > 1 && crate::gfx::array_layers(kept, gl) > max {
            kept -= 1;
        }
        if kept < slots {
            log::warn!("custom item textures: the GPU holds {kept} of {slots} glint textures");
        }
        let layers = crate::gfx::array_layers(kept, gl).min(max);
        let layer_bytes = textures.width as usize * textures.height as usize * 4;
        let mut data = std::borrow::Cow::Borrowed(&textures.pixels[..]);
        if layers != slots {
            let mut padded = textures.pixels[..layer_bytes * kept as usize].to_vec();
            padded.resize(layer_bytes * layers as usize, 0);
            data = std::borrow::Cow::Owned(padded);
        }
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("CIT glints"),
                size: wgpu::Extent3d {
                    width: textures.width,
                    height: textures.height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let linear = sampler("CIT glint blur", wgpu::FilterMode::Linear);
        let nearest = sampler("CIT glint", wgpu::FilterMode::Nearest);
        // hud.wgsl's `CitSlot`: rgba, then blur in x.
        let mut table = vec![[0.0f32; 8]; MAX_SLOTS];
        for (row, slot) in table.iter_mut().zip(&textures.slots) {
            row[..4].copy_from_slice(&slot.rgba);
            row[4] = if slot.blur { 1.0 } else { 0.0 };
        }
        let table = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CIT glint slots"),
            contents: bytemuck::cast_slice(&table),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let filtering = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CIT glint bindings"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                entry(1, filtering),
                entry(2, filtering),
                entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CIT glint resources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&linear),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: table.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon HUD (CIT glints)"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../hud.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CIT glint layout"),
            bind_group_layouts: &[Some(hud), Some(&layout)],
            ..Default::default()
        });
        let attributes =
            wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4,3=>Float32,4=>Float32x2];
        let pipelines = textures
            .blends
            .iter()
            .map(|&blend| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("CIT glint"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<HudVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &attributes,
                        })],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_cit_glint"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(blend_state(blend)),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();
        Some(Self { group, pipelines })
    }

    /// Draw `vertices` with blend `blend` (an index into `GlintTextures::blends`).
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        blend: u8,
        vertices: std::ops::Range<u32>,
    ) {
        if let Some(pipeline) = self.pipelines.get(usize::from(blend)) {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(vertices, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_optifine::Properties;
    use horizon_optifine::cit::parse_blend;

    fn rule(name: &str, text: &str) -> Rule {
        let path = format!("assets/minecraft/optifine/cit/{name}.properties");
        let text = format!("texture=glint\n{text}");
        Rule::parse(&path, &Properties::parse_equals_first(&text), |_| true).unwrap()
    }

    #[test]
    fn slots_share_one_size_and_their_blends() {
        let rules = [
            rule("a", "type=enchantment\nr=0.5"),
            rule("item", "items=stick"),
            rule("b", "type=enchantment\nblend=multiply\nblur=false"),
            rule("c", "type=enchantment"),
        ];
        // A 1x1 red glint and a 2x2 one: the red texel fills 2x2.
        let red = vec![255, 0, 0, 255];
        let quad: Vec<u8> = (0..16).collect();
        let t = GlintTextures::new(
            &rules,
            vec![
                (0, (1, 1, red)),
                (2, (2, 2, quad.clone())),
                (3, (2, 2, quad.clone())),
            ],
            MAX_BYTES,
        );
        assert_eq!((t.width, t.height), (2, 2));
        assert_eq!(t.slot_of, [Some(0), None, Some(1), Some(2)]);
        assert_eq!(&t.pixels[..16], [255, 0, 0, 255].repeat(4));
        assert_eq!(&t.pixels[16..32], &quad[..]);
        assert_eq!(
            t.blends,
            [
                parse_blend("add").unwrap(),
                parse_blend("multiply").unwrap()
            ]
        );
        let blends: Vec<_> = t.slots.iter().map(|s| s.blend).collect();
        assert_eq!(blends, [0, 1, 0]);
        assert_eq!(t.slots[0].rgba, [0.5, 1.0, 1.0, 1.0]);
        assert!(!t.slots[1].blur && t.slots[2].blur);
        assert!(GlintTextures::new(&rules, Vec::new(), MAX_BYTES).is_empty());
    }

    #[test]
    fn big_glints_are_scaled_down_to_the_largest_side() {
        let rules = [rule("wide", "type=enchantment\nblur=false")];
        // 2048 x 1, texel x holding x in red and green: every other texel is kept.
        let wide: Vec<u8> = (0..2048u32)
            .flat_map(|x| [(x % 256) as u8, (x / 256) as u8, 0, 255])
            .collect();
        let t = GlintTextures::new(&rules, vec![(0, (2048, 1, wide))], MAX_BYTES);
        assert_eq!((t.width, t.height), (1024, 1));
        for x in [0u32, 1, 300, 1023] {
            let at = x as usize * 4;
            let source = 2 * x;
            let want = [(source % 256) as u8, (source / 256) as u8, 0, 255];
            assert_eq!(t.pixels[at..at + 4], want, "texel {x}");
        }
    }

    #[test]
    fn the_array_is_halved_until_it_fits() {
        let sizes = |n: usize| std::iter::repeat_n((1024, 1024), n);
        // 16 layers of 1024 x 1024 RGBA are 64 MiB; a 17th halves both sides.
        assert_eq!(shared_size(sizes(16), 16, 64 << 20), Some((1024, 1024)));
        assert_eq!(shared_size(sizes(17), 17, 64 << 20), Some((512, 512)));
        // The largest of each side, never past MAX_SIDE.
        let mixed = [(16, 4096), (64, 32)].into_iter();
        assert_eq!(shared_size(mixed, 2, 64 << 20), Some((64, 1024)));
        assert_eq!(shared_size(std::iter::empty(), 0, 64 << 20), None);
    }

    #[test]
    fn blurred_glints_grow_smoothly_and_sharp_ones_by_texel() {
        // Black then white, doubled: the wrapping blend of the two, or each texel twice.
        let row = [0, 0, 0, 255, 255, 255, 255, 255];
        let red = |rgba: Vec<u8>| rgba.chunks(4).map(|t| t[0]).collect::<Vec<_>>();
        assert_eq!(red(resize(&row, (2, 1), (4, 1), true)), [64, 64, 191, 191]);
        assert_eq!(red(resize(&row, (2, 1), (4, 1), false)), [0, 0, 255, 255]);
    }

    #[test]
    fn a_layer_vertex_carries_its_slot_and_intensity() {
        // Unorm8x4 reads the colour's bytes in memory order: r, g, b, a.
        assert_eq!(vertex_color(3, 0.5).to_le_bytes(), [3, 0, 0, 128]);
        assert_eq!(vertex_color(200, 2.0).to_le_bytes(), [200, 0, 0, 255]);
        assert_eq!(vertex_color(0, -1.0).to_le_bytes(), [0, 0, 0, 0]);
    }

    #[test]
    fn a_still_layer_is_scaled_and_turned() {
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5;
        // No speed: no scroll whatever the clock.
        assert!(close(
            layer_uv([0.25, 0.5], 12_345.0, 0.0, 0.0, 8.0),
            [2.0, 4.0]
        ));
        // A quarter turn takes (1, 0) to (0, 1).
        assert!(close(layer_uv([1.0, 0.0], 0.0, 0.0, 90.0, 1.0), [0.0, 1.0]));
        // 1500 ms at speed 2: 3000 / 110000 back, 3000 / 30000 up.
        assert!(close(
            layer_uv([0.0, 0.0], 1500.0, 2.0, 0.0, 8.0),
            [-3.0 / 110.0, 0.1]
        ));
    }

    #[test]
    fn named_blends_take_their_gl_factors() {
        let multiply = blend_state(parse_blend("multiply").unwrap());
        assert_eq!(multiply.color.src_factor, wgpu::BlendFactor::Dst);
        assert_eq!(
            multiply.color.dst_factor,
            wgpu::BlendFactor::OneMinusSrcAlpha
        );
        // The alpha factors default to GL_ZERO, GL_ONE: the target's alpha stays.
        assert_eq!(multiply.alpha.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(multiply.alpha.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(
            blend_state(parse_blend("replace").unwrap()),
            wgpu::BlendState::REPLACE
        );
    }
}
