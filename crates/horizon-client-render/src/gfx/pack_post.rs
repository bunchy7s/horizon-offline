//! A shader pack's post-processing over Horizon's own world (spec 2.1 rows 4, 6, 8, 9; 6.1; 6.2;
//! 6.4): the world renders into its own target as with a resolution scale, its colour becomes
//! colortex0 and its depth `depthtex0`, then the pack's `prepare`, `deferred`, `composite` and
//! `final` programs run in order, and `final` draws onto the screen under the hand and HUD.
//! Gbuffers programs come later; until then the pack's composites read Horizon's picture.
//!
//! Every colortex buffer a program reads or writes has two textures, A and B. Programs read
//! "main" and write "alt", and each buffer a program wrote flips afterwards (6.1.6). The
//! mapping carries over to the next frame, so a buffer that is not cleared reads back what was
//! written last (OQ-8). Buffers are cleared at the start of the frame, "main" only.
//! Pack buffers store rows as GL does (row 0 at the bottom: the translator negates clip y), so
//! the world is copied in upside down, and its reverse-Z depth becomes GL window depth.

use std::collections::HashMap;

use glam::{DMat4, DVec3};
use horizon_shaders::{Format, Group, Kind, Post, Val};

use super::pack_draw::DrawUniforms;
use super::pack_entities::{PROGRAMS, PackEntities};
use super::pack_images;
use super::pack_sky::PackSky;
use super::pack_textures;
use super::pack_uniforms::{self, FrameUniforms, PackInputs};

const BUFFERS: usize = horizon_shaders::BUFFERS;

/// The texture behind a pack sampler name.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Color(usize),
    Depth,
    /// `depthtex1`/`depthtex2`: depth without translucent terrain (a copy taken before it).
    Depth1,
    Noise,
    /// A 1×1 stand-in for names this slice does not provide yet.
    Dummy,
    /// `texture.<stage>.<name>`: the pack's own texture, filtered and wrapped as its
    /// `.mcmeta` says.
    Custom {
        index: usize,
        blur: bool,
        clamp: bool,
    },
    /// `image.<name>` through its sampler name.
    Image(usize),
    /// A 1×1 depth of 1.0 behind a comparison sampler, until shadow maps exist: no shadow.
    Shadow,
    /// The block atlas (pack_atlas.rs), in gbuffers programs.
    Atlas,
    Lightmap,
    /// `shadowtex0`/`shadowtex1` (6.3), with depth comparison (`sampler2DShadow`) or not.
    ShadowDepth(usize, bool),
    /// `shadowcolor0/1` read outside the shadow programs (in them they are `Color`).
    ShadowColor(usize),
}

#[derive(Clone)]
struct Binding {
    source: Source,
    binding: u32,
    /// A filtering sampler (the texture is filterable).
    filtering: bool,
    /// The view's dimension: 2D, or 1D and 3D for custom textures and their stand-ins.
    dimension: wgpu::TextureViewDimension,
    /// Where this texture's sampler object is bound: the first texture of each kind binds it
    /// (`binding + 1`), the others share it (devices have as few as 16 sampler slots).
    sampler: u32,
}

/// The sampler objects a pass binds, one slot per kind it uses.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SamplerKind {
    Linear,
    Nearest,
    Repeat,
    NearestRepeat,
    Comparison,
}

impl SamplerKind {
    fn of(source: Source, filtering: bool) -> Self {
        match source {
            Source::Color(_) | Source::ShadowColor(_) | Source::Dummy if !filtering => {
                Self::Nearest
            }
            Source::Depth | Source::Depth1 | Source::ShadowDepth(_, false) => Self::Nearest,
            Source::Noise => Self::Repeat,
            Source::Shadow | Source::ShadowDepth(_, true) => Self::Comparison,
            // Blocks look as the game draws them: nearest texels.
            Source::Atlas => Self::NearestRepeat,
            Source::Image(_) if filtering => Self::Linear,
            Source::Image(_) => Self::Nearest,
            Source::Custom { blur, clamp, .. } => match (blur && filtering, clamp) {
                (true, true) => Self::Linear,
                (false, true) => Self::Nearest,
                (true, false) => Self::Repeat,
                (false, false) => Self::NearestRepeat,
            },
            Source::Color(_) | Source::ShadowColor(_) | Source::Dummy | Source::Lightmap => {
                Self::Linear
            }
        }
    }

    fn binding_type(self) -> wgpu::SamplerBindingType {
        match self {
            Self::Linear | Self::Repeat => wgpu::SamplerBindingType::Filtering,
            Self::Nearest | Self::NearestRepeat => wgpu::SamplerBindingType::NonFiltering,
            Self::Comparison => wgpu::SamplerBindingType::Comparison,
        }
    }
}

/// Points every sampler object of `module` at the slot `slot` gives its binding, so that
/// samplers sharing a slot become one variable (two variables may not share a binding).
fn share_samplers(module: &mut naga::Module, slot: &HashMap<u32, u32>) {
    let mut first = HashMap::new();
    let mut same = HashMap::new();
    for (h, g) in module.global_variables.iter_mut() {
        let Some(b) = g.binding.as_mut() else {
            continue;
        };
        if !matches!(module.types[g.ty].inner, naga::TypeInner::Sampler { .. }) {
            continue;
        }
        let Some(&to) = slot.get(&b.binding) else {
            continue;
        };
        b.binding = to;
        match first.entry(to) {
            std::collections::hash_map::Entry::Occupied(e) => _ = same.insert(h, *e.get()),
            std::collections::hash_map::Entry::Vacant(e) => _ = e.insert(h),
        }
    }
    if same.is_empty() {
        return;
    }
    let fix = |expressions: &mut naga::Arena<naga::Expression>| {
        for (_, e) in expressions.iter_mut() {
            if let naga::Expression::GlobalVariable(g) = e
                && let Some(&to) = same.get(g)
            {
                *g = to;
            }
        }
    };
    for (_, f) in module.functions.iter_mut() {
        fix(&mut f.expressions);
    }
    for ep in &mut module.entry_points {
        fix(&mut ep.function.expressions);
    }
}

/// The sky's geometry and the programs that draw it: `gbuffers_skybasic` (disc, sunset, stars,
/// void) and `gbuffers_skytextured` (sun, moon), each with its fallbacks. A part without a
/// program is Horizon's own.
struct Sky {
    geometry: PackSky,
    basic: Option<Pass>,
    textured: Option<Pass>,
}

/// Entities, block entities, translucent entity parts and particles a pack draws
/// (`pack_entities::ENTITY`... order), after terrain or, `late`, after deferred; Horizon draws
/// the others.
struct Entities {
    geometry: PackEntities,
    passes: [Option<Pass>; PROGRAMS],
    late: [bool; PROGRAMS],
    /// What draws entities and block entities into the shadow map (2.4).
    shadow: [Option<Pass>; 2],
    /// Only the player, only block entities that emit light (3.2).
    only: (bool, bool),
}

impl Entities {
    /// Whether a gbuffers program draws some of them (else only the shadow pass does).
    fn drawn(&self) -> bool {
        self.passes.iter().any(Option::is_some)
    }
}

/// What an image binds: a buffer (`colorimgN`, the side its sampler reads) or one of the pack's.
#[derive(Clone, Copy, PartialEq)]
enum Image {
    Color(usize),
    Custom(usize),
    /// The copy of a buffer or a pack image taken before a dispatch, which its loads read.
    ColorBefore(usize),
    CustomBefore(usize),
}

impl Image {
    fn buffer(self) -> Option<usize> {
        match self {
            Image::Color(b) => Some(b),
            _ => None,
        }
    }
}

/// The buffers and the pack images (masks) `images` read as copies taken before a dispatch.
fn befores(images: &[(u32, Image)]) -> (u16, u64) {
    images
        .iter()
        .fold((0, 0), |(colors, customs), i| match i.1 {
            Image::ColorBefore(b) => (colors | 1 << b, customs),
            Image::CustomBefore(k) => (colors, customs | 1 << k),
            _ => (colors, customs),
        })
}

/// The color buffers of `images`, as a mask.
fn image_buffers(images: &[(u32, Image)]) -> u16 {
    images
        .iter()
        .filter_map(|i| i.1.buffer())
        .fold(0, |m, b| m | 1 << b)
}

/// The format of the sink of passes that write only images.
const SINK: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// What packs use of the device beyond drawing. OpenGL ES and older drivers lack some of it: a
/// program that needs what is missing is skipped with a note, and the game runs on without it.
#[derive(Clone, Copy)]
struct Gpu {
    compute: bool,
    /// Images and storage buffers written by fragment shaders.
    fragment_storage: bool,
    /// Storage read and written by vertex shaders (a geometry stage).
    vertex_storage: bool,
    /// `clear_texture`; without it images are cleared by copying zeros.
    clear: bool,
    adapter_formats: bool,
}

impl Gpu {
    fn of(adapter: &wgpu::Adapter, features: wgpu::Features) -> Self {
        use wgpu::DownlevelFlags as D;
        let flags = adapter.get_downlevel_capabilities().flags;
        let mut gpu = Gpu {
            compute: flags.contains(D::COMPUTE_SHADERS),
            fragment_storage: flags.contains(D::FRAGMENT_WRITABLE_STORAGE),
            vertex_storage: flags.contains(D::VERTEX_STORAGE)
                && features.contains(wgpu::Features::VERTEX_WRITABLE_STORAGE),
            clear: features.contains(wgpu::Features::CLEAR_TEXTURE),
            adapter_formats: features
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES),
        };
        // `HORIZON_SHADER_PACK_WITHOUT=compute,fragment-storage,vertex-storage,clear` tries the
        // fallbacks on a device that has it all.
        let without = std::env::var("HORIZON_SHADER_PACK_WITHOUT").unwrap_or_default();
        for w in without.split(',') {
            match w.trim() {
                "compute" => gpu.compute = false,
                "fragment-storage" => gpu.fragment_storage = false,
                "vertex-storage" => gpu.vertex_storage = false,
                "clear" => gpu.clear = false,
                _ => {}
            }
        }
        gpu
    }

    /// Whether textures of `format` can be bound as images.
    fn storable(
        self,
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        features: wgpu::Features,
    ) -> bool {
        let usages = if self.adapter_formats {
            adapter.get_texture_format_features(format).allowed_usages
        } else {
            format.guaranteed_format_features(features).allowed_usages
        };
        (self.compute || self.fragment_storage)
            && usages.contains(wgpu::TextureUsages::STORAGE_BINDING)
    }

    /// The stages that may bind images and storage buffers.
    fn storage_stages(self) -> wgpu::ShaderStages {
        let mut stages = wgpu::ShaderStages::NONE;
        if self.compute {
            stages |= wgpu::ShaderStages::COMPUTE;
        }
        if self.fragment_storage {
            stages |= wgpu::ShaderStages::FRAGMENT;
        }
        if self.vertex_storage {
            stages |= wgpu::ShaderStages::VERTEX;
        }
        stages
    }
}

/// A pass's pipeline: a draw, or a compute dispatch.
#[derive(Clone)]
enum Pipeline {
    Render(wgpu::RenderPipeline),
    Compute {
        pipeline: wgpu::ComputePipeline,
        /// `workGroups`, else `workGroupsRender` and the local size.
        groups: Option<[u32; 3]>,
        render: [f32; 2],
        local: [u32; 3],
    },
}

#[derive(Clone)]
struct Pass {
    name: String,
    group: Group,
    pipeline: Pipeline,
    layout: wgpu::BindGroupLayout,
    ubo: wgpu::Buffer,
    block: Vec<u8>,
    /// (index into `pack_uniforms::NAMES`, declared type, offset).
    uniforms: Vec<(Option<usize>, String, u32)>,
    samplers: Vec<Binding>,
    /// (location, buffer) of each output that reaches a buffer.
    writes: Vec<(u32, usize)>,
    /// Buffers flipped after the pass.
    flip: u16,
    /// Buffers sampled, for the bind group cache key.
    reads: u16,
    /// shadowcolor buffers sampled by a program of the other buffers.
    shadow_reads: u16,
    /// Buffers written.
    written: u16,
    /// Buffers whose mip chain is regenerated right before it (6.1.7).
    mipmaps: u16,
    /// Reads the per-draw block (`DrawUniforms`).
    draw: bool,
    /// What `texture` reads instead of the block atlas.
    atlas: Option<wgpu::TextureView>,
    /// Reads `mc_Entity`'s table (`PackPost::block_ids`).
    ids: bool,
    /// Bind groups by the sides read (bits 0–15), the buffers whose mip chain is current (bits
    /// 16–31) and the shadowcolor sides read (bits 32–33).
    groups: HashMap<u64, wgpu::BindGroup>,
    /// (binding, target) of each image: for a buffer, the side its sampler reads.
    images: Vec<(u32, Image)>,
    /// Binds the pack's storage buffers in set 4.
    buffers: bool,
    /// Buffers both sampled and bound as images: sampled from a copy taken first, as wgpu binds
    /// a texture one way per pass.
    copies: u16,
    /// Compute programs dispatched before this pass.
    compute: Vec<Pass>,
    /// Writes no buffer, only images: draws into `Targets::sink`.
    sink: bool,
    /// Instances to draw for a geometry stage folded into the vertex stage; 0 without one.
    geometry: u32,
    /// The pack's images (bit n: `image.*` n) this pass samples and stores: sampled from a copy
    /// taken just before it.
    image_copies: u64,
    /// The pack's images a compute pass's loads read as they were before it: copied first.
    snapshots: u64,
}

impl Pass {
    fn render(&self) -> &wgpu::RenderPipeline {
        match &self.pipeline {
            Pipeline::Render(p) => p,
            Pipeline::Compute { .. } => unreachable!("a compute pass draws nothing"),
        }
    }
}

/// One side of a buffer with mipmaps.
struct Mips {
    /// Every level, for sampling.
    all: wgpu::TextureView,
    /// From the second level down: its attachment, and a bind group over the level above.
    levels: Vec<(wgpu::TextureView, wgpu::BindGroup)>,
}

struct Targets {
    size: [u32; 2],
    /// A and B of each buffer used.
    color: [Option<[wgpu::TextureView; 2]>; BUFFERS],
    depth: wgpu::TextureView,
    depth_texture: wgpu::Texture,
    /// `depthtex1`, when a program after translucent terrain reads it.
    depth1: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Both sides' mip chains of the buffers with mipmaps (`color` is their first level).
    mips: [Option<[Mips; 2]>; BUFFERS],
    /// Where a pass that both samples and stores a buffer samples it from.
    scratch: [Option<wgpu::TextureView>; BUFFERS],
    /// Each buffer's size: the render size, or its `size.buffer`.
    sizes: [[u32; 2]; BUFFERS],
    /// The attachment of passes that write only images.
    sink: wgpu::TextureView,
}

/// What a pack's gbuffers programs read and draw from the game's world.
pub(super) struct World<'a> {
    /// The block atlas and its size.
    pub atlas: Option<(wgpu::TextureView, [u32; 2])>,
    pub lightmap: wgpu::TextureView,
    /// Terrain's geometry, when the pack draws it.
    pub terrain: Option<Geometry<'a>>,
    /// `mc_Entity` of each block sprite layer: (id, 1 for fluids or -1) pairs.
    pub block_ids: &'a [i32],
    /// The atlas of the sun and moon's sprites.
    pub celestial: Option<wgpu::TextureView>,
    pub gui_size: [u32; 2],
    /// Set 1 of the clouds' draw: their offset and palette.
    pub clouds: Option<&'a wgpu::BindGroupLayout>,
    /// The game can hand its rain and snow to the pack.
    pub weather: bool,
    /// The game can hand its first-person hand to the pack.
    pub hand: bool,
}

/// This frame's clouds, when the pack draws them: their vertices, uniform, quad indices and
/// quad count.
pub(super) struct Clouds {
    pub vertices: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    pub indices: wgpu::Buffer,
    pub quads: u32,
}

/// This frame's rain and snow, when the pack draws them: their vertices, the quad indices, the
/// quads of each kind (rain, snow), each kind's sprite, and a number that changes with them.
pub(super) struct Weather {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub quads: [std::ops::Range<u32>; 2],
    pub sprites: [Option<wgpu::TextureView>; 2],
    pub generation: u64,
}

/// Shadow cache: the key of the shadow map the shadow targets hold. Only the
/// shadow passes write those targets, so while the key matches they still hold
/// it and the pass is skipped.
#[derive(Default)]
struct ShadowCache {
    key: Option<ShadowCacheKey>,
}

/// Key for validating the shadow cache.
/// Must match the cache state exactly to be considered valid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShadowCacheKey {
    sun_angle: f32,
    moon_angle: f32,
    snapped_camera: Option<DVec3>,
    shadow_settings_hash: u64,
    shadow_scene_revision: (u64, u64),
    ticks: u64,
    day_time: i64,
    shadow_uniforms_hash: u64,
}

pub struct PackPost {
    pub notes: Vec<String>,
    passes: Vec<Pass>,
    /// Shadow cache for temporal reprojection.
    shadow_cache: ShadowCache,
    /// What draws terrain (`gbuffers_terrain` or its fallback), between `prepare` and `deferred`.
    terrain: Option<Pass>,
    /// What draws translucent terrain (`gbuffers_water` or its fallback), after `deferred`.
    water: Option<Pass>,
    /// What draws the sky (`gbuffers_skybasic` or its fallback), before terrain, and its vertices.
    sky: Option<Sky>,
    /// What draws entities and block entities, after terrain, and their vertices.
    entities: Option<Entities>,
    /// What draws the clouds, after entities, and this frame's clouds.
    clouds: Option<Pass>,
    cloud_draw: Option<Clouds>,
    /// The pack has `gbuffers_clouds`, even one that writes nothing (a pack drawing its own
    /// clouds discards vanilla's): the game's clouds are not drawn.
    hides_clouds: bool,
    /// The pack has a sky (`gbuffers_skybasic`) or sun and moon (`gbuffers_skytextured`)
    /// program: vanilla's stays hidden even when the program writes nothing.
    hides_sky: bool,
    hides_celestial: bool,
    /// `colortex0ClearColor`, when the pack sets it.
    sky_clear: Option<[f32; 4]>,
    /// What draws rain and snow (the same program, each with its kind's sprite as `texture`),
    /// after translucent terrain, this frame's weather and the sprites' generation bound.
    weather: Option<[Pass; 2]>,
    weather_draw: Option<Weather>,
    weather_generation: u64,
    /// What draws the first-person hand (`gbuffers_hand` or its fallback), after the opaque
    /// world, and its vertices.
    hand: Option<(Pass, super::pack_hand::PackHand)>,
    /// A program after translucent terrain reads `depthtex1`: it is copied before water.
    depth1: bool,
    /// The shadow buffers, when the shadow pass runs (2.5), and the `shadowcomp` programs.
    shadow_targets: Option<super::pack_shadow::ShadowTargets>,
    /// What draws terrain from the light (`shadow`), and translucent terrain (`shadow_water`).
    shadow_terrain: Option<Pass>,
    shadow_water: Option<Pass>,
    shadow_comp: Vec<Pass>,
    /// Buffers with mipmaps (filterable ones a program asks them for).
    mipped: u16,
    /// Buffers whose mip chain matches their first level, this frame.
    mips_valid: u16,
    /// The downsampling pipeline of each format with mipmaps.
    down: HashMap<wgpu::TextureFormat, (wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
    /// The per-draw uniforms of gbuffers programs.
    draws: DrawUniforms,
    /// Set 3 of flat terrain (`mc_Entity`'s table) and its bind group for each arena page,
    /// with the page buffer it was made for.
    block_ids: Option<(wgpu::BindGroupLayout, wgpu::Buffer)>,
    page_groups: HashMap<u32, (wgpu::Buffer, wgpu::BindGroup)>,
    gui_size: [u32; 2],
    /// GPU time of each pass, while the renderer asks for it (`pack_timing.rs`).
    pub timing: Option<super::pack_timing::PassTiming>,
    /// `depthtex0` at the screen's centre, read back for `centerDepthSmooth`.
    center_depth: Option<super::pack_center_depth::CenterDepth>,
    /// `shaders.properties`' custom uniforms, after the frame's in `uniforms.values`.
    custom: horizon_shaders::Custom,
    atlas: Option<wgpu::TextureView>,
    lightmap: wgpu::TextureView,
    deferred_pre: u16,
    composite_pre: u16,
    formats: [wgpu::TextureFormat; BUFFERS],
    clears: [Option<[f32; 4]>; BUFFERS],
    used: u16,
    /// `size.buffer.*`.
    sizes: [Option<horizon_shaders::BufferSize>; BUFFERS],
    /// Buffers bound as images, and those also copied for a pass that samples them.
    storage: u16,
    copied: u16,
    /// Bit n set: B is buffer n's "main".
    main: u16,
    targets: Option<Targets>,
    noise: wgpu::TextureView,
    dummy: wgpu::TextureView,
    dummy_3d: wgpu::TextureView,
    dummy_1d: wgpu::TextureView,
    /// `image.*` and `bufferObject.*`.
    images: Vec<pack_images::PackImage>,
    /// Set 5 of programs with a geometry stage.
    indices: Option<wgpu::BindGroupLayout>,
    shadow_culling: horizon_shaders::ShadowCulling,
    gpu: Gpu,
    /// Zeros to clear images by copying, where the device has no `clear_texture`.
    zeros: Option<wgpu::Buffer>,
    dump: Option<pack_images::Dump>,
    buffers: pack_images::Buffers,
    textures: Vec<pack_textures::PackTexture>,
    /// The stand-in shadow map and its comparison sampler; cleared on the first frame.
    shadow: (wgpu::TextureView, wgpu::Sampler),
    shadow_cleared: bool,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    repeat: wgpu::Sampler,
    nearest_repeat: wgpu::Sampler,
    blit: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    /// The blit also writes depthtex0 (some program reads it) and clears these buffers.
    blit_depth: bool,
    blit_clears: Vec<usize>,
    /// Buffers cleared in passes of their own (more than the blit's eight attachments).
    other_clears: Vec<usize>,
    /// The blit's bindings, for the world target of this size.
    blit_group: Option<([u32; 2], wgpu::BindGroup)>,
    /// Shows colortex0 when the pack has no `final` (OQ-5).
    copy: (wgpu::RenderPipeline, wgpu::BindGroupLayout),
    uniforms: FrameUniforms,
}

/// The wgpu format of a pack format (6.7), the nearest one wgpu can render to.
fn texture_format(f: Format, features: wgpu::Features) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as T;
    let pick = |r, rg, rgba| match f.channels {
        1 => r,
        2 => rg,
        _ => rgba,
    };
    let half = pick(T::R16Float, T::Rg16Float, T::Rgba16Float);
    match (f.kind, f.bits) {
        (Kind::Unorm, 10) => T::Rgb10a2Unorm,
        (Kind::Unorm, 16)
            if features.contains(
                wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            ) =>
        {
            pick(T::R16Unorm, T::Rg16Unorm, T::Rgba16Unorm)
        }
        (Kind::Unorm, b) if b <= 8 => pick(T::R8Unorm, T::Rg8Unorm, T::Rgba8Unorm),
        (Kind::Float, 32) => pick(T::R32Float, T::Rg32Float, T::Rgba32Float),
        (Kind::Float, 11) if features.contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE) => {
            T::Rg11b10Ufloat
        }
        (Kind::Int, 8) => pick(T::R8Sint, T::Rg8Sint, T::Rgba8Sint),
        (Kind::Int, 16) => pick(T::R16Sint, T::Rg16Sint, T::Rgba16Sint),
        (Kind::Int, _) => pick(T::R32Sint, T::Rg32Sint, T::Rgba32Sint),
        (Kind::Uint, 8) => pick(T::R8Uint, T::Rg8Uint, T::Rgba8Uint),
        (Kind::Uint, 10) => T::Rgb10a2Uint,
        (Kind::Uint, 16) => pick(T::R16Uint, T::Rg16Uint, T::Rgba16Uint),
        (Kind::Uint, _) => pick(T::R32Uint, T::Rg32Uint, T::Rgba32Uint),
        // Snorm, 16-bit norm without the feature, RGB9_E5: half floats hold them.
        _ => half,
    }
}

/// The texture behind sampler `name` in a program of the shadow buffers (`shadow_space`: buffer
/// n is shadowcolor n) or of the others, when the shadow pass runs (`shadow_pass`). `shadow` is
/// `shadowtex1` when the program also declares `watershadow` (6.3).
/// Whether a `ty` sampler can read a custom texture of `sample_type`: custom textures hold
/// colours, never depth to compare against.
fn fits(ty: &str, sample_type: wgpu::TextureSampleType) -> bool {
    use wgpu::TextureSampleType as T;
    !ty.contains("Shadow")
        && match sample_type {
            T::Sint => ty.starts_with('i'),
            T::Uint => ty.starts_with('u'),
            _ => ty.starts_with('s'),
        }
}

fn source(name: &str, shadow_space: bool, shadow_pass: bool, watershadow: bool) -> Source {
    let shadow_color = match name {
        "shadowcolor" | "shadowcolor0" => Some(0),
        "shadowcolor1" => Some(1),
        _ => None,
    };
    let shadow_depth = match name {
        "shadowtex0" | "watershadow" | "shadowtex0HW" => Some(0),
        "shadowtex1" | "shadowtex1HW" => Some(1),
        "shadow" => Some(usize::from(watershadow)),
        _ => None,
    };
    if shadow_pass {
        if let Some(n) = shadow_depth {
            return Source::ShadowDepth(n, name.ends_with("HW"));
        }
        if let Some(n) = shadow_color {
            return if shadow_space {
                Source::Color(n)
            } else {
                Source::ShadowColor(n)
            };
        }
    }
    if shadow_space && horizon_shaders::buffer_index(name).is_some() {
        // colortex buffers are not this pass's.
        return Source::Dummy;
    }
    match name {
        "depthtex0" | "gdepthtex" => Source::Depth,
        // The hand is drawn after the pack, so `depthtex2` (without it) is `depthtex1`.
        "depthtex1" | "depthtex2" => Source::Depth1,
        "noisetex" => Source::Noise,
        "texture" | "gtexture" | "tex" => Source::Atlas,
        "lightmap" => Source::Lightmap,
        n => horizon_shaders::buffer_index(n).map_or(Source::Dummy, Source::Color),
    }
}

/// 256×256 RGB white noise (6.4), from a fixed seed so every run looks the same.
fn noise(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let texels: Vec<u8> = (0..256 * 256 * 4)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if i % 4 == 3 { 255 } else { (state >> 56) as u8 }
        })
        .collect();
    texture(device, queue, "Shader pack noisetex", 256, &texels)
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    side: u32,
    texels: &[u8],
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: side,
        height: side,
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        tex.as_image_copy(),
        texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: None,
        },
        size,
    );
    tex.create_view(&Default::default())
}

fn sampler(
    device: &wgpu::Device,
    filter: wgpu::FilterMode,
    address: wgpu::AddressMode,
) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Shader pack"),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    })
}

/// Runs `create` and returns its validation error, if any, or the backend's (a shader the
/// driver's compiler refuses is an internal error). A panic while it runs is an error too:
/// naga's backends turn a module they cannot write into a panic as the driver gets the pipeline
/// (#3525), which a pack's program must not turn into a crash.
fn checked<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let value = crate::crash::contain(create);
    let internal = pollster::block_on(internal.pop());
    match pollster::block_on(validation.pop()).or(internal) {
        Some(e) => {
            // The details are in the error's description or its sources.
            let mut message = match &e {
                wgpu::Error::Validation { source, .. } => format!("{source:?}"),
                e => e.to_string(),
            };
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                message = format!("{message}: {s}");
                source = s.source();
            }
            Err(message)
        }
        None => value.map_err(|panic| format!("the shader compiler failed: {panic}")),
    }
}

/// A full-screen helper pipeline of ours (`pack_post.wgsl`).
fn helper(
    device: &wgpu::Device,
    vertex: &str,
    entry: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
    targets: &[Option<wgpu::ColorTargetState>],
    depth: Option<wgpu::DepthStencilState>,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Shader pack helpers"),
        source: wgpu::ShaderSource::Wgsl(include_str!("pack_post.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(entry),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: depth,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    texture_entry_of(binding, sample_type, wgpu::TextureViewDimension::D2)
}

fn texture_entry_of(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        },
    )
}

/// `blend.<program>`'s factors as wgpu's, adding both terms.
fn blend_state(b: horizon_shaders::Blend) -> wgpu::BlendState {
    use horizon_shaders::Factor as F;
    use wgpu::BlendFactor as W;
    let f = |f| match f {
        F::Zero => W::Zero,
        F::One => W::One,
        F::SrcColor => W::Src,
        F::OneMinusSrcColor => W::OneMinusSrc,
        F::DstColor => W::Dst,
        F::OneMinusDstColor => W::OneMinusDst,
        F::SrcAlpha => W::SrcAlpha,
        F::OneMinusSrcAlpha => W::OneMinusSrcAlpha,
        F::DstAlpha => W::DstAlpha,
        F::OneMinusDstAlpha => W::OneMinusDstAlpha,
        F::SrcAlphaSaturate => W::SrcAlphaSaturated,
    };
    let component = |src, dst| wgpu::BlendComponent {
        src_factor: f(src),
        dst_factor: f(dst),
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component(b[0], b[1]),
        alpha: component(b[2], b[3]),
    }
}

fn target(format: wgpu::TextureFormat) -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })
}

fn attach(
    view: &wgpu::TextureView,
    ops: wgpu::Operations<wgpu::Color>,
) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops,
    })
}

/// Which of the frame's terrain draws: the camera's solid or translucent ones, or the shadow
/// camera's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TerrainList {
    Solid,
    Translucent,
    ShadowSolid,
    ShadowTranslucent,
}

impl TerrainList {
    fn translucent(self) -> bool {
        matches!(self, Self::Translucent | Self::ShadowTranslucent)
    }
}

/// Draws the game's terrain of one list into a pass; calls its third argument with each arena
/// page before drawing from it.
/// Draws a list's terrain: indexed, or for a program with a geometry stage, without indices and
/// with that many instances (and its index buffer bound in set 5 by this layout).
pub(super) type TerrainDraw<'a> = dyn FnMut(
        &mut wgpu::RenderPass<'_>,
        TerrainList,
        &dyn Fn(&mut wgpu::RenderPass<'_>, u32),
        Option<(u32, &wgpu::BindGroupLayout)>,
    ) + 'a;

/// What the shadow camera sees of the terrain, for the game's draw lists: its view and
/// projection in wgpu's clip space, the direction toward the light, and how far from the camera
/// terrain is drawn (`shadowDistanceRenderMul`).
pub(super) struct ShadowView {
    pub cull: glam::Mat4,
    pub light: glam::Vec3,
    pub distance: Option<f32>,
    /// Draw everything, every face, within this many blocks (`shadow.culling=false|reversed`):
    /// packs that voxelize the world in their shadow pass need all of it.
    pub unculled: Option<f32>,
}

/// Set 3 of flat terrain, read by the vertex stage: `mc_Entity`'s table (-1 pairs when empty)
/// and, per arena page, the page's vertices (a vertex reads its quad's corners for its normal).
fn block_ids(device: &wgpu::Device, table: &[i32]) -> (wgpu::BindGroupLayout, wgpu::Buffer) {
    let empty = [-1, -1];
    let table = if table.is_empty() { &empty[..] } else { table };
    let buffer = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("Shader pack block ids"),
            contents: bytemuck::cast_slice(table),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Shader pack terrain"),
        entries: &[storage(0), storage(1)],
    });
    (layout, buffer)
}

/// The game's geometry a gbuffers program draws: its vertex buffers and, for flat draws
/// (flat.rs), the layout of their bind group 1.
pub(super) struct Geometry<'a> {
    pub buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    pub table: Option<&'a wgpu::BindGroupLayout>,
    /// Writes depth (the sky does not).
    pub depth_write: bool,
    /// What `texture` reads instead of the block atlas.
    pub atlas: Option<wgpu::TextureView>,
    /// `mc_Entity`'s table at set 3 (flat terrain).
    pub ids: Option<&'a wgpu::BindGroupLayout>,
    /// Back faces are culled, as for the game's own terrain.
    pub cull: bool,
}

/// What a pack brings besides its programs: custom textures, images and storage buffers.
struct Resources<'a> {
    textures: &'a [pack_textures::PackTexture],
    images: &'a [pack_images::PackImage],
    /// The images this device can't make.
    left_out: &'a [horizon_shaders::CustomImage],
    buffers: &'a pack_images::Buffers,
    /// Set 5 of a program with a geometry stage: the draw's indices.
    indices: Option<&'a wgpu::BindGroupLayout>,
    gpu: Gpu,
    /// The buffers whose format can be bound as an image.
    storable: u16,
}

/// A program's bind group layout (its uniform block, samplers and images) and how each sampler
/// and image binds. `None` (with a note) for an image the game can't give it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn bindings(
    features: wgpu::Features,
    interface: &horizon_shaders::Interface,
    name: &str,
    group: Group,
    buffers: &[u32],
    geometry: Option<&Geometry<'_>>,
    formats: &[wgpu::TextureFormat; BUFFERS],
    shadow: Option<&[wgpu::TextureFormat; 2]>,
    notes: &mut Vec<String>,
    pack: &Resources<'_>,
) -> Option<(
    Vec<wgpu::BindGroupLayoutEntry>,
    Vec<Binding>,
    HashMap<u32, u32>,
    u16,
    u16,
    Vec<(u32, Image)>,
)> {
    let textures = pack.textures;
    if let Some(b) = interface.buffers.iter().find(|&&b| !pack.buffers.has(b)) {
        notes.push(format!("{name}: buffer {b} has no `bufferObject.{b}`"));
        return None;
    }
    let mut entries = vec![entry(
        0,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )];
    let mut samplers = Vec::new();
    let (mut kinds, mut slots) = (HashMap::new(), HashMap::new());
    let mut reads = 0u16;
    let mut shadow_reads = 0u16;
    let watershadow = interface.samplers.iter().any(|s| s.name == "watershadow");
    // A gbuffers pass draws into its buffers and the depth buffer: reading them as well would
    // be a feedback loop (undefined in GL, refused by wgpu), so those read a blank texture.
    let in_place = match geometry {
        Some(_) => buffers.iter().fold(0u16, |m, &b| m | 1 << b),
        None => 0,
    };
    for s in &interface.samplers {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        // A stand-in of the sampler's own dimension.
        let stand_in = if s.ty.contains("1D") {
            wgpu::TextureViewDimension::D1
        } else if s.ty.contains("3D") {
            wgpu::TextureViewDimension::D3
        } else {
            wgpu::TextureViewDimension::D2
        };
        if let Some(def) = pack
            .left_out
            .iter()
            .find(|d| d.sampler.as_deref() == Some(s.name.as_str()))
        {
            notes.push(format!(
                "{name}: samples image `{}`, which is left out",
                def.name
            ));
            return None;
        }
        let custom = pack_textures::find(textures, group, &s.ty, &s.name);
        let image = pack
            .images
            .iter()
            .position(|i| i.def.sampler.as_deref() == Some(s.name.as_str()));
        let (src, sample_type, dimension) = match custom {
            _ if let Some(k) = image => {
                let i = &pack.images[k];
                (Source::Image(k), i.sample_type(features), i.dimension)
            }
            Some(i) if !fits(&s.ty, textures[i].sample_type) => {
                notes.push(format!(
                    "{name}: `{} {}` can't read texture.{}.{}",
                    s.ty, s.name, textures[i].stage, textures[i].name
                ));
                (Source::Dummy, float, stand_in)
            }
            Some(i) => {
                let t = &textures[i];
                let src = match t.image {
                    pack_textures::Image::Lightmap => Source::Lightmap,
                    pack_textures::Image::Atlas => Source::Atlas,
                    pack_textures::Image::View(_) => Source::Custom {
                        index: i,
                        blur: t.blur,
                        clamp: t.clamp,
                    },
                };
                (src, t.sample_type, t.dimension)
            }
            None => {
                let mut src = source(&s.name, group.shadow(), shadow.is_some(), watershadow);
                let blank = match src {
                    // The shadow pass writes the shadow map.
                    Source::ShadowDepth(..) => group == Group::Shadow,
                    Source::Atlas => geometry.is_none(),
                    Source::Depth => geometry.is_some(),
                    // Translucent terrain reads the copy; opaque terrain would read itself.
                    Source::Depth1 => group == Group::Gbuffers,
                    Source::Color(n) => in_place & 1 << n != 0,
                    _ => false,
                };
                let mut sample_type = match src {
                    Source::Color(n) => formats[n]
                        .sample_type(None, Some(features))
                        .unwrap_or(float),
                    Source::ShadowColor(n) => shadow
                        .and_then(|f| f[n].sample_type(None, Some(features)))
                        .unwrap_or(float),
                    Source::Depth | Source::Depth1 | Source::ShadowDepth(..) => {
                        wgpu::TextureSampleType::Float { filterable: false }
                    }
                    // Unfilterable on WebGPU-level devices; the lightmap is sampled at texel centres.
                    Source::Lightmap => float,
                    _ => float,
                };
                let int = !matches!(sample_type, wgpu::TextureSampleType::Float { .. });
                if s.ty == "sampler2DShadow" {
                    src = match src {
                        Source::ShadowDepth(n, _) => Source::ShadowDepth(n, true),
                        _ => Source::Shadow,
                    };
                    sample_type = wgpu::TextureSampleType::Depth;
                }
                let wanted = match s.ty.as_str() {
                    "sampler2DShadow" => true,
                    "sampler2D" => !int,
                    "isampler2D" => sample_type == wgpu::TextureSampleType::Sint,
                    "usampler2D" => sample_type == wgpu::TextureSampleType::Uint,
                    _ => false,
                };
                if !wanted {
                    notes.push(format!(
                        "{}: `{} {}` is not provided yet",
                        name, s.ty, s.name
                    ));
                    (src, sample_type) = (Source::Dummy, float);
                }
                if blank && sample_type == wgpu::TextureSampleType::Depth {
                    src = Source::Shadow;
                } else if blank {
                    // The blank texture is float: an integer sampler then fails the pipeline (a note).
                    src = Source::Dummy;
                    if int {
                        sample_type = float;
                    }
                }
                let dimension = match src {
                    Source::Dummy => stand_in,
                    _ => wgpu::TextureViewDimension::D2,
                };
                (src, sample_type, dimension)
            }
        };
        let filtering = sample_type == float;
        match src {
            Source::Color(n) => reads |= 1 << n,
            Source::ShadowColor(n) => shadow_reads |= 1 << n,
            _ => {}
        }
        entries.push(texture_entry_of(s.binding, sample_type, dimension));
        let kind = SamplerKind::of(src, filtering);
        let sampler = *kinds.entry(kind).or_insert_with(|| {
            entries.push(entry(
                s.binding + 1,
                wgpu::BindingType::Sampler(kind.binding_type()),
            ));
            s.binding + 1
        });
        slots.insert(s.binding + 1, sampler);
        samplers.push(Binding {
            source: src,
            binding: s.binding,
            filtering,
            sampler,
            dimension,
        });
    }
    let mut images = Vec::new();
    for i in &interface.images {
        let buffer = i
            .name
            .strip_prefix("colorimg")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&n| n < BUFFERS && !group.shadow());
        let own = pack.images.iter().position(|c| c.def.name == i.name);
        let (target, actual) = match (buffer, own) {
            (_, Some(k)) => (Image::Custom(k), pack.images[k].format),
            (Some(b), None) => (Image::Color(b), formats[b]),
            (None, None) if i.before.is_some() => {
                // Bound as the image it copies.
                let of = i.before.as_deref().unwrap_or_default();
                let buffer = of
                    .strip_prefix("colorimg")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|&n| n < BUFFERS && !group.shadow());
                match (buffer, pack.images.iter().position(|c| c.def.name == of)) {
                    (_, Some(k)) => (Image::CustomBefore(k), pack.images[k].format),
                    (Some(b), None) => (Image::ColorBefore(b), formats[b]),
                    (None, None) => {
                        notes.push(format!("{name}: image `{of}` is not provided yet"));
                        return None;
                    }
                }
            }
            (None, None) => {
                notes.push(format!("{name}: image `{}` is not provided yet", i.name));
                return None;
            }
        };
        let Some(format) = image_format(&i.format) else {
            notes.push(format!("{name}: `{}` has format {}", i.name, i.format));
            return None;
        };
        if actual != format {
            notes.push(format!(
                "{name}: `{}` is {} but its texture is {actual:?}",
                i.name, i.format
            ));
            return None;
        }
        let access = match i.access.as_str() {
            "writeonly" => wgpu::StorageTextureAccess::WriteOnly,
            "readonly" => wgpu::StorageTextureAccess::ReadOnly,
            "atomic" => wgpu::StorageTextureAccess::Atomic,
            _ => wgpu::StorageTextureAccess::ReadWrite,
        };
        let dimension = if i.ty.ends_with("3D") {
            wgpu::TextureViewDimension::D3
        } else {
            wgpu::TextureViewDimension::D2
        };
        if let Image::Color(b) | Image::ColorBefore(b) = target
            && pack.storable & 1 << b == 0
        {
            notes.push(format!(
                "{name}: `{}` can't be an image on this GPU; skipped",
                i.name
            ));
            return None;
        }
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: i.binding,
            visibility: pack.gpu.storage_stages(),
            ty: wgpu::BindingType::StorageTexture {
                access,
                format,
                view_dimension: dimension,
            },
            count: None,
        });
        images.push((i.binding, target));
    }
    Some((entries, samplers, slots, reads, shadow_reads, images))
}

/// The wgpu format of an image's format qualifier.
fn image_format(qualifier: &str) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as T;
    Some(match qualifier {
        "rgba32f" => T::Rgba32Float,
        "rgba16f" => T::Rgba16Float,
        "rg32f" => T::Rg32Float,
        "rg16f" => T::Rg16Float,
        "r11f_g11f_b10f" => T::Rg11b10Ufloat,
        "r32f" => T::R32Float,
        "r16f" => T::R16Float,
        "rgba16" => T::Rgba16Unorm,
        "rgb10_a2" => T::Rgb10a2Unorm,
        "rgba8" => T::Rgba8Unorm,
        "rg16" => T::Rg16Unorm,
        "rg8" => T::Rg8Unorm,
        "r16" => T::R16Unorm,
        "r8" => T::R8Unorm,
        "rgba16_snorm" => T::Rgba16Snorm,
        "rgba8_snorm" => T::Rgba8Snorm,
        "rg16_snorm" => T::Rg16Snorm,
        "rg8_snorm" => T::Rg8Snorm,
        "r16_snorm" => T::R16Snorm,
        "r8_snorm" => T::R8Snorm,
        "rgba32i" => T::Rgba32Sint,
        "rgba16i" => T::Rgba16Sint,
        "rgba8i" => T::Rgba8Sint,
        "rg32i" => T::Rg32Sint,
        "rg16i" => T::Rg16Sint,
        "rg8i" => T::Rg8Sint,
        "r32i" => T::R32Sint,
        "r16i" => T::R16Sint,
        "r8i" => T::R8Sint,
        "rgba32ui" => T::Rgba32Uint,
        "rgba16ui" => T::Rgba16Uint,
        "rgb10_a2ui" => T::Rgb10a2Uint,
        "rgba8ui" => T::Rgba8Uint,
        "rg32ui" => T::Rg32Uint,
        "rg16ui" => T::Rg16Uint,
        "rg8ui" => T::Rg8Uint,
        "r32ui" => T::R32Uint,
        "r16ui" => T::R16Uint,
        "r8ui" => T::R8Uint,
        _ => return None,
    })
}

/// The pass of program `p`: a quad, or `geometry` drawn by the game. `None` (with a note) when
/// wgpu refuses it or it writes nothing. `formats` are its buffers' (for a program of the shadow
/// buffers, buffer n is shadowcolor n); `shadow` the shadowcolor formats when the shadow pass
/// runs.
#[allow(clippy::too_many_arguments)]
fn build_pass(
    device: &wgpu::Device,
    p: horizon_shaders::PostProgram,
    formats: &[wgpu::TextureFormat; BUFFERS],
    shadow: Option<&[wgpu::TextureFormat; 2]>,
    screen: wgpu::TextureFormat,
    notes: &mut Vec<String>,
    geometry: Option<&Geometry<'_>>,
    custom: &[String],
    draws: &wgpu::BindGroupLayout,
    pack: &Resources<'_>,
) -> Option<Pass> {
    let features = device.features();
    // The frame's uniforms, then the pack's custom ones.
    let index = |name: &str| {
        pack_uniforms::index(name).or_else(|| {
            let k = custom.iter().position(|c| c == name)?;
            Some(pack_uniforms::NAMES.len() + k)
        })
    };
    let mask = |flips: &[(u32, bool)]| flips.iter().filter(|f| f.1).fold(0u16, |m, f| m | 1 << f.0);
    let mut p = p;
    let program = p.program.take()?;
    let interface = &program.interface;
    let draw = interface.draw;
    let storage = !interface.images.is_empty() || !interface.buffers.is_empty();
    if storage && !pack.gpu.fragment_storage {
        notes.push(format!(
            "{}: this GPU can't write images from a fragment shader; skipped",
            p.name
        ));
        return None;
    }
    if p.geometry_triangles > 0 && (pack.indices.is_none() || !pack.gpu.vertex_storage) {
        notes.push(format!(
            "{}: this GPU can't run a geometry stage; skipped",
            p.name
        ));
        return None;
    }
    let (entries, samplers, slots, reads, shadow_reads, images) = bindings(
        features, interface, &p.name, p.group, &p.buffers, geometry, formats, shadow, notes, pack,
    )?;
    let final_pass = p.group == Group::Final;
    let mut targets = Vec::new();
    let mut writes = Vec::new();
    for &location in &interface.outputs {
        let l = location as usize;
        if targets.len() <= l {
            targets.resize(l + 1, None);
        }
        if final_pass {
            if location == 0 {
                targets[0] = target(screen);
            }
        } else if let Some(&buffer) = p.buffers.get(l) {
            let format = formats[buffer as usize];
            let mut state = target(format);
            if let (Some(t), Some(b)) = (
                &mut state,
                p.buffer_blends
                    .iter()
                    .find(|(n, _)| *n == buffer)
                    .map_or(p.blend, |(_, b)| *b),
            ) && format
                .guaranteed_format_features(features)
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
            {
                t.blend = Some(blend_state(b));
            }
            targets[l] = state;
            writes.push((location, buffer as usize));
        }
    }
    // Outputs past the last buffer the program writes go nowhere, as in GL; a pass has no
    // attachment for them.
    while targets.last().is_some_and(Option::is_none) {
        targets.pop();
    }
    // A program that only writes images draws into a sink that keeps nothing; one drawing
    // geometry has the depth buffer, and draws with no colour target at all.
    let sink = !final_pass && writes.is_empty() && !program.interface.images.is_empty();
    if sink && geometry.is_none() {
        targets = vec![Some(wgpu::ColorTargetState {
            format: SINK,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        })];
    } else if !final_pass && writes.is_empty() && !sink {
        notes.push(format!("{}: writes no buffer; skipped", p.name));
        return None;
    }
    let written = writes.iter().fold(0u16, |m, w| m | 1 << w.1);
    let kept = p
        .flips
        .iter()
        .filter(|f| !f.1)
        .fold(0u16, |m, f| m | 1 << f.0);
    // Gbuffers programs write in place (6.1.6).
    let flip = if final_pass || geometry.is_some() {
        0
    } else {
        (written & !kept) | mask(&p.flips)
    };
    let created = checked(device, || {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&p.name),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&p.name),
            bind_group_layouts: &[
                Some(&layout),
                geometry.and_then(|g| g.table),
                draw.then_some(draws),
                geometry.and_then(|g| g.ids),
                (!interface.buffers.is_empty()).then_some(&pack.buffers.layout),
                pack.indices.filter(|_| p.geometry_triangles > 0),
            ],
            ..Default::default()
        });
        let module = |mut m: naga::Module| {
            share_samplers(&mut m, &slots);
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&p.name),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(m)),
            })
        };
        let (vs, fs) = (module(program.vertex), module(program.fragment));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&p.name),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                // A geometry stage reads its vertices itself.
                buffers: geometry
                    .filter(|_| p.geometry_triangles == 0)
                    .map_or(&[], |g| g.buffers),
            },
            // Rows flipped to GL's turn front faces clockwise.
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: geometry
                    .filter(|g| g.cull && p.geometry_triangles == 0)
                    .map(|_| wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: geometry.map(|g| wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(g.depth_write),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: if p.name == "gbuffers_damagedblock" {
                    wgpu::DepthBiasState {
                        constant: -10,
                        slope_scale: -1.0,
                        clamp: 0.0,
                    }
                } else {
                    Default::default()
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fs,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        (layout, pipeline)
    });
    let (layout, pipeline) = match created {
        Ok(created) => created,
        Err(e) => {
            notes.push(format!("{}: skipped: {e}", p.name));
            return None;
        }
    };
    let interface = &program.interface;
    let missing: Vec<&str> = interface
        .uniforms
        .iter()
        .filter(|u| !u.name.starts_with("hz_") && index(&u.name).is_none())
        .map(|u| u.name.as_str())
        .collect();
    if !missing.is_empty() {
        notes.push(format!("{}: zero for now: {}", p.name, missing.join(" ")));
    }
    let image_copies = image_copies(&samplers, &images);
    Some(Pass {
        ubo: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&p.name),
            size: u64::from(interface.uniform_size.max(16)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        block: vec![0; interface.uniform_size as usize],
        uniforms: interface
            .uniforms
            .iter()
            .map(|u| (index(&u.name), u.ty.clone(), u.offset))
            .collect(),
        name: p.name,
        group: p.group,
        pipeline: Pipeline::Render(pipeline),
        layout,
        samplers,
        writes,
        flip,
        shadow_reads,
        written,
        mipmaps: p.mipmaps,
        draw,
        atlas: geometry.and_then(|g| g.atlas.clone()),
        ids: geometry.is_some_and(|g| g.ids.is_some()),
        groups: HashMap::new(),
        copies: image_buffers(&images) & reads,
        reads: reads | image_buffers(&images),
        buffers: !interface.buffers.is_empty(),
        images,
        compute: Vec::new(),
        sink,
        geometry: p.geometry_triangles,
        image_copies,
        snapshots: 0,
    })
}

/// The pack's images a pass both samples and stores.
fn image_copies(samplers: &[Binding], images: &[(u32, Image)]) -> u64 {
    samplers
        .iter()
        .filter_map(|s| match s.source {
            Source::Image(k) if images.iter().any(|i| i.1 == Image::Custom(k)) => Some(k),
            _ => None,
        })
        .fold(0, |m, k| m | 1 << k)
}

/// The pass of compute program `c` of a `group` program: `None` (with a note) when wgpu refuses
/// it or it needs what the game can't give.
#[allow(clippy::too_many_arguments)]
fn build_compute(
    device: &wgpu::Device,
    name: String,
    group: Group,
    c: horizon_shaders::ComputeProgram,
    formats: &[wgpu::TextureFormat; BUFFERS],
    shadow: Option<&[wgpu::TextureFormat; 2]>,
    notes: &mut Vec<String>,
    custom: &[String],
    pack: &Resources<'_>,
) -> Option<Pass> {
    let index = |name: &str| {
        pack_uniforms::index(name).or_else(|| {
            let k = custom.iter().position(|c| c == name)?;
            Some(pack_uniforms::NAMES.len() + k)
        })
    };
    let interface = &c.interface;
    let (entries, samplers, slots, reads, shadow_reads, images) = bindings(
        device.features(),
        interface,
        &name,
        group,
        &[],
        None,
        formats,
        shadow,
        notes,
        pack,
    )?;
    let local = c.module.entry_points[0].workgroup_size;
    let created = checked(device, || {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&name),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&name),
            bind_group_layouts: &[
                Some(&layout),
                None,
                None,
                None,
                (!interface.buffers.is_empty()).then_some(&pack.buffers.layout),
            ],
            ..Default::default()
        });
        let mut module = c.module;
        share_samplers(&mut module, &slots);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&name),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&name),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        (layout, pipeline)
    });
    let (layout, pipeline) = match created {
        Ok(created) => created,
        Err(e) => {
            notes.push(format!("{name}: skipped: {e}"));
            return None;
        }
    };
    let written = image_buffers(&images);
    let image_copies = image_copies(&samplers, &images);
    let (before_colors, snapshots) = befores(&images);
    Some(Pass {
        ubo: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&name),
            size: u64::from(interface.uniform_size.max(16)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        block: vec![0; interface.uniform_size as usize],
        uniforms: interface
            .uniforms
            .iter()
            .map(|u| (index(&u.name), u.ty.clone(), u.offset))
            .collect(),
        name,
        group,
        pipeline: Pipeline::Compute {
            pipeline,
            groups: c.work_groups,
            render: c.work_groups_render,
            local,
        },
        layout,
        samplers,
        writes: Vec::new(),
        flip: 0,
        copies: written & reads | before_colors,
        reads: reads | written,
        shadow_reads,
        written,
        mipmaps: 0,
        draw: false,
        atlas: None,
        ids: false,
        groups: HashMap::new(),
        buffers: !interface.buffers.is_empty(),
        images,
        compute: Vec::new(),
        sink: false,
        geometry: 0,
        image_copies,
        snapshots,
    })
}

impl PackPost {
    /// Builds the pipelines of `post` for a screen of `screen` format.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
        mut post: Post,
        screen: wgpu::TextureFormat,
        world: World<'_>,
    ) -> Self {
        let features = device.features();
        let formats: [wgpu::TextureFormat; BUFFERS] =
            std::array::from_fn(|i| texture_format(post.config.buffers[i].format, features));
        let sky_clear = Some(&post.config.buffers[0])
            .filter(|b| b.clear)
            .and_then(|b| b.clear_color);
        let clears = std::array::from_fn(|i| {
            let b = &post.config.buffers[i];
            b.clear.then(|| {
                b.clear_color
                    .unwrap_or(if i == 1 { [1.0; 4] } else { [0.0; 4] })
            })
        });
        let mut notes = post.notes;
        notes.extend(post.config.notes);
        let mut sizes = post.sizes;
        // The blit writes colortex0 beside the depth buffer, at the render size.
        if sizes[0].take().is_some() {
            notes.push("size.buffer.colortex0: colortex0 keeps the render size".into());
        }
        let textures = pack_textures::upload(device, queue, post.textures, &mut notes);
        let shadow_config = post.config.shadow.clone();
        let mut custom = post.custom;
        let custom_names: Vec<String> = custom.uniforms().map(|u| u.0.to_owned()).collect();
        // A first run finds the names neither the frame nor the pack defines.
        custom.eval(0.0, &|n| pack_uniforms::index(n).map(|_| Val::scalar(0.0)));
        // Its zeros are no start for `smooth`: the first frame's values are, as in Iris.
        custom.restart_smoothing();
        if !custom.notes.is_empty() {
            notes.push(format!("custom uniforms: {}", custom.notes.join("; ")));
            custom.notes.clear();
        }
        let mask =
            |flips: &[(u32, bool)]| flips.iter().filter(|f| f.1).fold(0u16, |m, f| m | 1 << f.0);
        let mut used = 1u16;
        let draws = DrawUniforms::new(device);
        // The shadow buffers' formats when the shadow pass runs; their programs see them as
        // buffers 0 and 1.
        let shadow_formats: Option<[wgpu::TextureFormat; 2]> = post.shadow_pass.then(|| {
            std::array::from_fn(|n| texture_format(shadow_config.colors[n].format, features))
        });
        let shadow_space: [wgpu::TextureFormat; BUFFERS] = std::array::from_fn(|i| {
            shadow_formats
                .filter(|_| i < 2)
                .map_or(formats[i], |f| f[i])
        });
        let (mut storage, mut copied, mut image_copied, mut snapshotted) = (0u16, 0u16, 0u64, 0u64);
        let gpu = Gpu::of(adapter, features);
        if !gpu.compute {
            notes.push("this GPU has no compute shaders: compute programs are skipped".into());
        }
        // An image this device can't make or store into: the programs using it are skipped.
        let mut image_defs = std::mem::take(&mut post.images);
        let mut left_out = Vec::new();
        image_defs.retain(|def| {
            let made = pack_images::makeable(device, def).and_then(|()| {
                if gpu.storable(adapter, pack_images::format(def.format), features) {
                    Ok(())
                } else {
                    Err("this GPU can't store into its format".into())
                }
            });
            if let Err(e) = &made {
                notes.push(format!("image `{}`: {e}", def.name));
                left_out.push(def.clone());
            }
            made.is_ok()
        });
        // A mask of a pass's images holds this many.
        let kept = image_defs.len().min(pack_images::IMAGES);
        for def in image_defs.drain(kept..) {
            notes.push(format!(
                "image `{}`: past the {} images a pack may have",
                def.name,
                pack_images::IMAGES
            ));
            left_out.push(def);
        }
        let images = pack_images::images(image_defs);
        let mut objects = std::mem::take(&mut post.buffer_objects);
        if gpu.storage_stages().is_empty() {
            objects.clear();
        }
        let buffers = pack_images::Buffers::new(device, objects, gpu.storage_stages());
        let storable = (0..BUFFERS)
            .filter(|&i| gpu.storable(adapter, formats[i], features))
            .fold(0u16, |m, i| m | 1 << i);
        let indices = gpu.vertex_storage.then(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Shader pack geometry indices"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            })
        });
        let pack = Resources {
            textures: &textures,
            images: &images,
            left_out: &left_out,
            buffers: &buffers,
            indices: indices.as_ref(),
            gpu,
            storable,
        };
        let mut build = |mut p: horizon_shaders::PostProgram, geometry: Option<&Geometry<'_>>| {
            let in_shadow = p.group.shadow();
            let mut compute = std::mem::take(&mut p.compute);
            if !gpu.compute {
                compute.clear();
                p.program.as_ref()?;
            }
            let group = p.group;
            let mut computes: Vec<Pass> = compute
                .into_iter()
                .filter_map(|(name, c)| {
                    build_compute(
                        device,
                        name,
                        group,
                        c,
                        if in_shadow { &shadow_space } else { &formats },
                        shadow_formats.as_ref(),
                        &mut notes,
                        &custom_names,
                        &pack,
                    )
                })
                .collect();
            let mut pass = if p.program.is_none() {
                // Compute stages only: the last is the pass, the others run before it.
                let mut last = computes.pop()?;
                last.compute = computes;
                computes = Vec::new();
                last
            } else {
                build_pass(
                    device,
                    p,
                    if in_shadow { &shadow_space } else { &formats },
                    shadow_formats.as_ref(),
                    screen,
                    &mut notes,
                    geometry,
                    &custom_names,
                    &draws.layout,
                    &pack,
                )?
            };
            pass.compute.append(&mut computes);
            // A pass's attachments share a size: the depth buffer's, for the passes that draw
            // the world.
            let size =
                |b: usize| sizes[b].unwrap_or(horizon_shaders::BufferSize::Relative([1.0; 2]));
            let depth = matches!(group, Group::Gbuffers | Group::Translucent);
            let mut written = pass.writes.iter().map(|w| size(w.1));
            let first = if depth {
                Some(horizon_shaders::BufferSize::Relative([1.0; 2]))
            } else {
                written.next()
            };
            if !in_shadow && group != Group::Final && written.any(|s| Some(s) != first) {
                notes.push(format!(
                    "{}: writes buffers of different sizes (size.buffer); skipped",
                    pass.name
                ));
                return None;
            }
            for p in std::iter::once(&pass).chain(&pass.compute) {
                if !in_shadow {
                    used |= p.reads | p.written | p.flip;
                }
                storage |= image_buffers(&p.images);
                image_copied |= p.image_copies;
                snapshotted |= p.snapshots;
                copied |= p.copies;
            }
            Some(pass)
        };
        // Flat terrain reads `mc_Entity` from a table of the sprite layers.
        let block_ids = world
            .terrain
            .as_ref()
            .filter(|g| g.table.is_some())
            .map(|_| block_ids(device, world.block_ids));
        let terrain_geometry = world.terrain.as_ref().map(|g| Geometry {
            buffers: g.buffers,
            table: g.table,
            depth_write: g.depth_write,
            atlas: g.atlas.clone(),
            ids: block_ids.as_ref().map(|i| &i.0),
            cull: g.cull,
        });
        let terrain = post
            .terrain
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let water = post
            .water
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let shadow_terrain = post
            .shadow
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let shadow_water = post
            .shadow_water
            .zip(terrain_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        let (hides_sky, hides_celestial) = (post.sky.is_some(), post.sky_textured.is_some());
        let sky = (post.sky.is_some() || post.sky_textured.is_some()).then(|| {
            let geometry = PackSky::new(device);
            let basic = Geometry {
                buffers: &PackSky::buffers(),
                table: Some(&geometry.layout),
                depth_write: false,
                atlas: None,
                ids: None,
                cull: false,
            };
            let basic = post.sky.and_then(|p| build(p, Some(&basic)));
            let textured = Geometry {
                buffers: &PackSky::celestial_buffers(),
                table: None,
                depth_write: false,
                atlas: world.celestial.clone(),
                ids: None,
                cull: false,
            };
            let textured = post.sky_textured.and_then(|p| build(p, Some(&textured)));
            Sky {
                geometry,
                basic,
                textured,
            }
        });
        let sky = sky.filter(|s| s.basic.is_some() || s.textured.is_some());
        let programs = [
            post.entities,
            post.block,
            post.entities_translucent,
            post.particles,
            post.damaged_block,
        ];
        let late = [
            false,
            false,
            post.translucent_late,
            post.particles_late,
            false,
        ];
        let shadow_only = (post.shadow_only_player, post.shadow_only_lit_blocks);
        let shadow_programs = [post.shadow_entities, post.shadow_block];
        let entity_vertices = (programs.iter().chain(&shadow_programs))
            .any(Option::is_some)
            .then(|| PackEntities::new(device, &draws.layout));
        let entity_buffers = super::pack_entities::buffers();
        let entity_geometry = entity_vertices.as_ref().map(|e| Geometry {
            buffers: &entity_buffers,
            table: Some(&e.table),
            depth_write: true,
            atlas: world.celestial.clone(),
            ids: None,
            cull: false,
        });
        let entity_passes = programs.map(|p| {
            p.zip(entity_geometry.as_ref()).and_then(|(p, g)| {
                if p.name == "gbuffers_damagedblock" {
                    let decal = Geometry {
                        buffers: g.buffers,
                        table: g.table,
                        depth_write: false,
                        atlas: g.atlas.clone(),
                        ids: g.ids,
                        cull: false,
                    };
                    build(p, Some(&decal))
                } else {
                    build(p, Some(g))
                }
            })
        });
        let shadow_entities = shadow_programs.map(|p| {
            p.zip(entity_geometry.as_ref())
                .and_then(|(p, g)| build(p, Some(g)))
        });
        let entities = entity_vertices
            .filter(|_| (entity_passes.iter().chain(&shadow_entities)).any(Option::is_some))
            .map(|geometry| Entities {
                geometry,
                passes: entity_passes,
                late,
                shadow: shadow_entities,
                only: shadow_only,
            });
        let cloud_geometry = world.clouds.map(|layout| Geometry {
            buffers: &entity_buffers,
            table: Some(layout),
            depth_write: true,
            atlas: world.celestial.clone(),
            ids: None,
            cull: false,
        });
        let hides_clouds = post.clouds.is_some() && cloud_geometry.is_some();
        let weather_geometry = world.weather.then(|| Geometry {
            buffers: &entity_buffers,
            table: None,
            depth_write: true,
            atlas: None,
            ids: None,
            cull: false,
        });
        let hand_geometry = world.hand.then(|| super::pack_hand::PackHand::new(device));
        let hand = post.hand.zip(hand_geometry).and_then(|(p, geometry)| {
            let g = Geometry {
                buffers: &entity_buffers,
                table: Some(&geometry.layout),
                depth_write: true,
                atlas: world.celestial.clone(),
                ids: None,
                cull: false,
            };
            build(p, Some(&g)).map(|pass| (pass, geometry))
        });
        let weather = post
            .weather
            .zip(weather_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)))
            .map(|rain| {
                let mut snow = rain.clone();
                snow.name = format!("{} (snow)", rain.name);
                [rain, snow]
            });
        let clouds = post
            .clouds
            .zip(cloud_geometry.as_ref())
            .and_then(|(p, g)| build(p, Some(g)));
        // Developers' aid: `HORIZON_SHADER_PACK_SKIP=composite,final` leaves programs out.
        let skip = std::env::var("HORIZON_SHADER_PACK_SKIP").unwrap_or_default();
        let passes: Vec<Pass> = post
            .programs
            .into_iter()
            .filter(|p| !skip.split(',').any(|s| s == p.name))
            .filter_map(|p| build(p, None))
            .collect();
        let shadow_comp: Vec<Pass> = post
            .shadow_comp
            .into_iter()
            .filter(|p| !skip.split(',').any(|s| s == p.name))
            .filter_map(|p| build(p, None))
            .collect();
        if let Some(t) = &terrain {
            notes.push(format!("terrain draws with {}", t.name));
        }
        for (part, pass) in [
            ("terrain", &shadow_terrain),
            ("translucent terrain", &shadow_water),
        ] {
            if let Some(p) = pass {
                notes.push(format!("{part} casts shadows with {}", p.name));
            }
        }
        if let Some(w) = &water {
            notes.push(format!("translucent terrain draws with {}", w.name));
        }
        // Timed (and labelled) apart from the solid shadow pass that runs the same program.
        let mut shadow_water = shadow_water;
        if let Some(p) = &mut shadow_water {
            p.name.push_str(" (translucent)");
        }
        for (part, pass) in sky
            .iter()
            .flat_map(|s| [("sky", &s.basic), ("sun and moon", &s.textured)])
        {
            if let Some(p) = pass {
                notes.push(format!("the {part} draw with {}", p.name));
            }
        }
        let parts = [
            "entities",
            "block entities",
            "translucent entities",
            "particles",
        ];
        if let Some(c) = &clouds {
            notes.push(format!("clouds draw with {}", c.name));
        }
        if let Some((h, _)) = &hand {
            notes.push(format!("the hand draws with {}", h.name));
        }
        if let Some([w, _]) = &weather {
            notes.push(format!("rain and snow draw with {}", w.name));
        }
        for (part, pass) in entities.iter().flat_map(|e| parts.iter().zip(&e.passes)) {
            if let Some(p) = pass {
                notes.push(format!("{part} draw with {}", p.name));
            }
        }
        // Timed (and labelled) apart from the terrain's shadow pass, which may run the same
        // program.
        let mut entities = entities;
        for e in entities.iter_mut() {
            let only = [
                (e.only.0, " (only the player)"),
                (e.only.1, " (only those that emit light)"),
            ];
            for ((part, pass), (only, which)) in parts.iter().zip(&mut e.shadow).zip(only) {
                if let Some(p) = pass {
                    let which = if only { which } else { "" };
                    notes.push(format!("{part}{which} cast shadows with {}", p.name));
                    p.name.push_str(&format!(" ({part})"));
                }
            }
        }
        let depth1 = water.is_some()
            && passes
                .iter()
                .filter(|p| matches!(p.group, Group::Composite | Group::Final))
                .chain(&water)
                .chain(weather.iter().take(1).flatten())
                .any(|p| p.samplers.iter().any(|s| s.source == Source::Depth1));
        let every_pass = || {
            (passes
                .iter()
                .chain(&shadow_comp)
                .chain(&terrain)
                .chain(&water))
            .chain(shadow_terrain.iter().chain(&shadow_water))
            .chain(sky.iter().flat_map(|s| s.basic.iter().chain(&s.textured)))
            .chain(entities.iter().flat_map(|e| e.passes.iter().flatten()))
            .chain(entities.iter().flat_map(|e| e.shadow.iter().flatten()))
            .chain(&clouds)
            .chain(weather.iter().flatten())
            .chain(hand.iter().map(|h| &h.0))
        };
        let shadow_targets = shadow_formats.map(|f| {
            let colors = every_pass().fold(0u16, |m, p| {
                let own = if p.group.shadow() {
                    p.reads | p.written | p.flip
                } else {
                    0
                };
                m | own | p.shadow_reads
            });
            let depth1 = every_pass()
                .flat_map(|p| &p.samplers)
                .any(|s| matches!(s.source, Source::ShadowDepth(1, _)));
            let t =
                super::pack_shadow::ShadowTargets::new(device, &shadow_config, f, colors, depth1);
            notes.push(format!(
                "shadow map {0}x{0} over {2} blocks, sunPathRotation {3}, shadowcolor buffers {colors:#b}{1}",
                t.size,
                if depth1 { ", shadowtex1" } else { "" },
                shadow_config.distance,
                shadow_config.sun_path_rotation,
            ));
            t
        });
        // A clear only matters when a program reads the buffer before one writes all of it
        // (composite passes cover their whole target, terrain does not); the others are skipped.
        let mut cleared = Vec::new();
        for b in 1..BUFFERS {
            let bit = 1u16 << b;
            let touched = |p: &&Pass| (p.reads | p.flip | p.written) & bit != 0;
            // Gbuffers passes cover part of their targets: a clear is needed if they touch it.
            let first = sky
                .iter()
                .flat_map(|s| s.basic.iter().chain(&s.textured))
                .chain(&terrain)
                .chain(entities.iter().flat_map(|e| e.passes.iter().flatten()))
                .chain(&clouds)
                .chain(&water)
                .chain(weather.iter().take(1).flatten())
                .chain(hand.iter().map(|h| &h.0))
                .find(touched)
                .map(|_| true)
                .or_else(|| {
                    let p = passes.iter().find(touched)?;
                    Some(p.reads & bit != 0 || p.written & bit == 0)
                });
            if clears[b].is_some() && first == Some(true) {
                cleared.push(b);
            }
        }
        // Terrain tests against the world's depth (entities...), so the blit copies it then.
        let blit_depth = terrain.is_some()
            || water.is_some()
            || sky.is_some()
            || entities.as_ref().is_some_and(Entities::drawn)
            || clouds.is_some()
            || weather.is_some()
            || hand.is_some()
            || passes.iter().any(|p| {
                p.samplers
                    .iter()
                    .any(|s| matches!(s.source, Source::Depth | Source::Depth1))
            });
        // Buffers of a size of their own clear apart from the blit, which writes colortex0.
        let (mut cleared, mut other_clears): (Vec<usize>, Vec<usize>) =
            cleared.into_iter().partition(|&b| sizes[b].is_none());
        let room = 8 - 1;
        other_clears.extend(cleared.split_off(room.min(cleared.len())));
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let blit_targets = std::iter::once(target(formats[0]));
        // Cleared by the pass's load operation; the shader writes nothing there.
        let blit_targets: Vec<_> = blit_targets
            .chain(cleared.iter().map(|&b| {
                Some(wgpu::ColorTargetState {
                    format: formats[b],
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })
            }))
            .collect();
        let blit = helper(
            device,
            "vs_full",
            if blit_depth {
                "fs_blit"
            } else {
                "fs_blit_color"
            },
            &[
                texture_entry(0, float),
                texture_entry(1, wgpu::TextureSampleType::Depth),
            ],
            &blit_targets,
            // GL's window depth, written for depthtex0 and the terrain's depth test.
            blit_depth.then_some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let copy = helper(
            device,
            "vs_uv",
            "fs_copy",
            &[
                texture_entry(0, float),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
            &[target(screen)],
            None,
        );
        let filterable = |b: usize| {
            formats[b].sample_type(None, Some(device.features()))
                == Some(wgpu::TextureSampleType::Float { filterable: true })
        };
        let asked = passes.iter().fold(0u16, |m, p| m | p.mipmaps);
        let mipped = (0..BUFFERS)
            .filter(|&b| asked & 1 << b != 0 && filterable(b))
            .fold(0u16, |m, b| m | 1 << b);
        if asked & !mipped != 0 {
            notes.push(format!(
                "mipmaps of unfilterable buffers are not generated (buffers {:#b})",
                asked & !mipped
            ));
        }
        let mut down = HashMap::new();
        for b in (0..BUFFERS).filter(|b| mipped & 1 << b != 0) {
            down.entry(formats[b]).or_insert_with(|| {
                helper(
                    device,
                    "vs_full",
                    "fs_down",
                    &[
                        texture_entry(0, float),
                        entry(
                            2,
                            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        ),
                    ],
                    &[target(formats[b])],
                    None,
                )
            });
        }
        let white = [255u8; 4];
        Self {
            notes,
            passes,
            terrain,
            water,
            sky,
            entities,
            clouds,
            cloud_draw: None,
            hides_clouds,
            hides_sky,
            hides_celestial,
            sky_clear,
            weather,
            weather_draw: None,
            weather_generation: 0,
            hand,
            depth1,
            shadow_targets,
            shadow_terrain,
            shadow_water,
            shadow_comp,
            mipped,
            mips_valid: 0,
            down,
            draws,
            block_ids,
            page_groups: HashMap::new(),
            gui_size: world.gui_size,
            timing: None,
            center_depth: None,
            custom,
            atlas: world.atlas.as_ref().map(|a| a.0.clone()),
            lightmap: world.lightmap,
            deferred_pre: mask(&post.deferred_pre),
            composite_pre: mask(&post.composite_pre),
            formats,
            clears,
            used,
            storage,
            copied,
            sizes,
            blit_depth,
            blit_clears: cleared,
            other_clears,
            blit_group: None,
            main: 0,
            targets: None,
            noise: pack_textures::noise(&textures)
                .cloned()
                .unwrap_or_else(|| noise(device, queue)),
            dummy: texture(device, queue, "Shader pack stand-in", 1, &white),
            images: {
                let mut images = images;
                for (k, image) in images.iter_mut().enumerate() {
                    image.copied = image_copied & 1 << k != 0;
                    image.snapshotted = snapshotted & 1 << k != 0;
                }
                images
            },
            buffers,
            indices,
            shadow_culling: post.shadow_culling,
            gpu,
            zeros: None,
            dump: pack_images::Dump::new(),
            dummy_3d: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Shader pack 3D stand-in"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
            dummy_1d: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Shader pack 1D stand-in"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D1,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
            textures,
            shadow: (
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Shader pack shadow stand-in"),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Depth32Float,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default()),
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Shader pack shadow"),
                    compare: Some(wgpu::CompareFunction::LessEqual),
                    ..Default::default()
                }),
            ),
            shadow_cleared: false,
            linear: sampler(
                device,
                wgpu::FilterMode::Linear,
                wgpu::AddressMode::ClampToEdge,
            ),
            nearest: sampler(
                device,
                wgpu::FilterMode::Nearest,
                wgpu::AddressMode::ClampToEdge,
            ),
            repeat: sampler(device, wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat),
            nearest_repeat: sampler(device, wgpu::FilterMode::Nearest, wgpu::AddressMode::Repeat),
            blit,
            copy,
            uniforms: {
                let mut u = FrameUniforms::default();
                u.atlas = world.atlas.map_or([0; 2], |a| a.1);
                u.shadow = shadow_config;
                u
            },
            shadow_cache: Default::default(),
        }
    }

    /// Writes `pass`'s uniforms for a target of `out` pixels; its bind group for the buffers'
    /// current mapping. The buffers it both samples and stores to are copied first, and it
    /// samples the copies.
    fn bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &mut Pass,
        out: [u32; 2],
    ) -> wgpu::BindGroup {
        pack_images::copy(encoder, &self.images, pass.image_copies);
        if let Some(targets) = &self.targets {
            for i in (0..BUFFERS).filter(|i| pass.copies & 1 << i != 0) {
                if let (Some(t), Some(copy)) = (&targets.color[i], &targets.scratch[i]) {
                    let side = usize::from(self.main & 1 << i != 0);
                    encoder.copy_texture_to_texture(
                        t[side].texture().as_image_copy(),
                        copy.texture().as_image_copy(),
                        t[side].texture().size(),
                    );
                }
            }
        }
        let fog = self.uniforms.fog();
        // Our block members first (`hz_target`, `hz_fog_color`, `hz_fog`), then the pack's.
        let ours = [
            [
                out[0] as f32,
                out[1] as f32,
                1.0 / out[0] as f32,
                1.0 / out[1] as f32,
            ],
            fog[0].to_array(),
            fog[1].to_array(),
        ];
        let head: &[u8] = bytemuck::cast_slice(&ours);
        let n = head.len().min(pass.block.len());
        pass.block[..n].copy_from_slice(&head[..n]);
        for (index, ty, offset) in &pass.uniforms {
            if let Some(v) = index.and_then(|i| self.uniforms.values.get(i)) {
                v.write(ty, &mut pass.block, *offset as usize);
            }
        }
        if !pass.block.is_empty() {
            queue.write_buffer(&pass.ubo, 0, &pass.block);
        }
        let targets = self.targets.as_ref().expect("resized");
        let shadow = self.shadow_targets.as_ref();
        let shadow_main = shadow.map_or(0, |s| s.main);
        let shadow_space = pass.group.shadow();
        // A chain is sampled only while it is current: a buffer written since its last chain
        // reads its first level, never stale levels (6.1.7). The shadow buffers have none.
        let (main, current) = if shadow_space {
            (shadow_main, 0)
        } else {
            (self.main, self.mips_valid & self.mipped & pass.reads)
        };
        let key = u64::from(main & pass.reads)
            | u64::from(current) << 16
            | u64::from(shadow_main & pass.shadow_reads) << 32;
        let group = pass.groups.entry(key).or_insert_with(|| {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: pass.ubo.as_entire_binding(),
            }];
            for s in &pass.samplers {
                let shadow_color = |i: usize, side: usize| {
                    shadow.and_then(|t| t.color[i].as_ref()).map(|c| &c[side])
                };
                let texture = match s.source {
                    Source::Color(i) if shadow_space => {
                        shadow_color(i, usize::from(key & 1 << i != 0)).unwrap_or(&self.dummy)
                    }
                    Source::ShadowColor(i) => {
                        shadow_color(i, usize::from(key & 1 << (32 + i) != 0))
                            .unwrap_or(&self.dummy)
                    }
                    Source::ShadowDepth(i, _) => match shadow {
                        Some(t) => match (i, &t.depth1) {
                            (1, Some(d)) => &d.1,
                            _ => &t.depth.1,
                        },
                        None => &self.shadow.0,
                    },
                    Source::Color(i) if pass.copies & 1 << i != 0 => {
                        targets.scratch[i].as_ref().unwrap_or(&self.dummy)
                    }
                    Source::Color(i) => {
                        let side = usize::from(key & 1 << i != 0);
                        match (&targets.mips[i], &targets.color[i]) {
                            (Some(m), _) if current & 1 << i != 0 => &m[side].all,
                            (_, Some(t)) => &t[side],
                            (_, None) => &self.dummy,
                        }
                    }
                    Source::Depth => &targets.depth,
                    // Before translucent terrain the two are the same.
                    Source::Depth1 => match &targets.depth1 {
                        Some(d)
                            if matches!(
                                pass.group,
                                Group::Composite | Group::Final | Group::Translucent
                            ) =>
                        {
                            &d.1
                        }
                        _ => &targets.depth,
                    },
                    Source::Noise => &self.noise,
                    Source::Dummy if s.dimension == wgpu::TextureViewDimension::D3 => {
                        &self.dummy_3d
                    }
                    Source::Dummy if s.dimension == wgpu::TextureViewDimension::D1 => {
                        &self.dummy_1d
                    }
                    Source::Dummy => &self.dummy,
                    Source::Image(k) if pass.image_copies & 1 << k != 0 => {
                        self.images[k].copy.as_ref().unwrap_or(&self.dummy)
                    }
                    Source::Image(k) => self.images[k].view.as_ref().unwrap_or(&self.dummy),
                    Source::Custom { index, .. } => match &self.textures[index].image {
                        pack_textures::Image::View(v) => v,
                        _ => &self.dummy,
                    },
                    Source::Shadow => &self.shadow.0,
                    Source::Atlas => pass
                        .atlas
                        .as_ref()
                        .or(self.atlas.as_ref())
                        .unwrap_or(&self.dummy),
                    Source::Lightmap => &self.lightmap,
                };
                let sampler = match SamplerKind::of(s.source, s.filtering) {
                    SamplerKind::Linear => &self.linear,
                    SamplerKind::Nearest => &self.nearest,
                    SamplerKind::Repeat => &self.repeat,
                    SamplerKind::NearestRepeat => &self.nearest_repeat,
                    SamplerKind::Comparison => shadow.map_or(&self.shadow.1, |t| &t.comparison),
                };
                entries.push(wgpu::BindGroupEntry {
                    binding: s.binding,
                    resource: wgpu::BindingResource::TextureView(texture),
                });
                if s.sampler == s.binding + 1 {
                    entries.push(wgpu::BindGroupEntry {
                        binding: s.sampler,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    });
                }
            }
            // An image is the side its buffer's sampler reads.
            for &(binding, image) in &pass.images {
                let view = match image {
                    Image::Color(i) => {
                        let side = usize::from(key & 1 << i != 0);
                        targets.color[i].as_ref().map_or(&self.dummy, |t| &t[side])
                    }
                    Image::Custom(k) => self.images[k].view.as_ref().unwrap_or(&self.dummy),
                    Image::ColorBefore(i) => targets.scratch[i].as_ref().unwrap_or(&self.dummy),
                    Image::CustomBefore(k) => self.images[k].before.as_ref().unwrap_or(&self.dummy),
                };
                entries.push(wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&pass.name),
                layout: &pass.layout,
                entries: &entries,
            })
        });
        group.clone()
    }

    /// Dispatches compute pass `pass` over a target of `size`.
    fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &mut Pass,
        size: [u32; 2],
    ) {
        let Pipeline::Compute {
            pipeline,
            groups,
            render,
            local,
        } = &pass.pipeline
        else {
            return;
        };
        let groups = groups.unwrap_or_else(|| {
            let n = |k: usize| (size[k] as f32 * render[k] / local[k] as f32).ceil() as u32;
            [n(0), n(1), 1]
        });
        let (pipeline, groups) = (pipeline.clone(), groups);
        for image in (0..self.images.len()).filter(|k| pass.snapshots & 1 << k != 0) {
            pack_images::snapshot(encoder, &self.images[image]);
        }
        let group = self.bind(device, queue, encoder, pass, size);
        let mut cp = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(&pass.name),
            timestamp_writes: None,
        });
        cp.set_pipeline(&pipeline);
        cp.set_bind_group(0, &group, &[]);
        if pass.buffers
            && let Some(g) = &self.buffers.group
        {
            cp.set_bind_group(4, g, &[]);
        }
        cp.dispatch_workgroups(groups[0], groups[1], groups[2]);
        drop(cp);
        self.mips_valid &= !pass.written;
    }

    /// A render pass into the buffers `pass` writes, in place, tested against the world's depth.
    fn begin<'e>(
        &mut self,
        encoder: &'e mut wgpu::CommandEncoder,
        pass: &Pass,
    ) -> wgpu::RenderPass<'e> {
        let targets = self.targets.as_ref().expect("resized");
        let load = wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        };
        // The shadow pass draws into the shadow buffers and the shadow map, the frame's first
        // one clearing them.
        let (depth_ops, shadow_ops) = match &mut self.shadow_targets {
            Some(s) if pass.group.shadow() => s.clear_ops(pass.writes.iter().map(|w| w.1)),
            _ => (super::pack_shadow::load(), [load; 2]),
        };
        let shadow = self.shadow_targets.as_ref().filter(|_| pass.group.shadow());
        let mut attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = Vec::new();
        for &(location, buffer) in &pass.writes {
            let l = location as usize;
            if attachments.len() <= l {
                attachments.resize(l + 1, None);
            }
            attachments[l] = match shadow {
                Some(s) => s
                    .color(buffer, false)
                    .and_then(|v| attach(v, shadow_ops.get(buffer).copied().unwrap_or(load))),
                None => targets.color[buffer]
                    .as_ref()
                    .and_then(|t| attach(&t[usize::from(self.main & 1 << buffer != 0)], load)),
            };
        }
        let timestamp_writes = self.timing.as_mut().and_then(|t| t.writes(&pass.name));
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&pass.name),
            color_attachments: &attachments,
            timestamp_writes,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: shadow.map_or(&targets.depth, |s| &s.depth.1),
                depth_ops: Some(depth_ops),
                stencil_ops: None,
            }),
            ..Default::default()
        })
    }

    /// Terrain (or, `translucent`, translucent terrain) with the pack's program; `draw` issues
    /// the game's draws.
    fn draw_terrain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        list: TerrainList,
        draw: &mut TerrainDraw<'_>,
    ) {
        let slot = match list {
            TerrainList::Solid => &mut self.terrain,
            TerrainList::Translucent => &mut self.water,
            TerrainList::ShadowSolid => &mut self.shadow_terrain,
            TerrainList::ShadowTranslucent => &mut self.shadow_water,
        };
        let Some(mut pass) = slot.take() else {
            return;
        };
        let translucent = list.translucent();
        if list == TerrainList::Translucent
            && let Some(t) = &self.targets
            && let Some((copy, _)) = &t.depth1
        {
            encoder.copy_texture_to_texture(
                t.depth_texture.as_image_copy(),
                copy.as_image_copy(),
                t.depth_texture.size(),
            );
        }
        let group = self.bind(device, queue, encoder, &mut pass, size);
        {
            let mut rp = self.begin(encoder, &pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            if pass.draw {
                let stage = if translucent {
                    "TERRAIN_TRANSLUCENT"
                } else {
                    "TERRAIN_SOLID"
                };
                self.draws.bind(&mut rp, stage);
            }
            let (groups, ids) = (&self.page_groups, pass.ids);
            let page = |rp: &mut wgpu::RenderPass<'_>, page: u32| {
                if let Some((_, group)) = groups.get(&page).filter(|_| ids) {
                    rp.set_bind_group(3, group, &[]);
                }
            };
            let geometry = self
                .indices
                .as_ref()
                .filter(|_| pass.geometry > 0)
                .map(|indices| (pass.geometry, indices));
            draw(&mut rp, list, &page, geometry);
        }
        if !pass.group.shadow() {
            self.mips_valid &= !pass.written;
        }
        *match list {
            TerrainList::Solid => &mut self.terrain,
            TerrainList::Translucent => &mut self.water,
            TerrainList::ShadowSolid => &mut self.shadow_terrain,
            TerrainList::ShadowTranslucent => &mut self.shadow_water,
        } = Some(pass);
    }

    /// The sky's parts in vanilla's order, each with the program that draws it; one render pass
    /// while consecutive programs write the same buffers.
    fn draw_sky(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        frame: &super::Frame,
    ) {
        let Some(mut sky) = self.sky.take() else {
            return;
        };
        let parts = sky
            .geometry
            .prepare(queue, frame, self.uniforms.shadow.sun_path_rotation);
        let mut groups = [None, None];
        for (i, pass) in [&mut sky.basic, &mut sky.textured].into_iter().enumerate() {
            if let Some(pass) = pass {
                groups[i] = Some(self.bind(device, queue, encoder, pass, size));
                self.mips_valid &= !pass.written;
            }
        }
        let passes = [&sky.basic, &sky.textured];
        let mut open: Option<(wgpu::RenderPass<'_>, usize)> = None;
        for part in parts {
            let i = usize::from(part.textured);
            let (Some(pass), Some(group)) = (passes[i], &groups[i]) else {
                continue;
            };
            let current = open.as_ref().map(|o| o.1);
            if current != Some(i) {
                let same = current
                    .and_then(|c| passes[c].as_ref())
                    .is_some_and(|p| p.writes == pass.writes);
                if !same {
                    drop(open.take());
                    open = Some((self.begin(encoder, pass).forget_lifetime(), i));
                }
                let rp = &mut open.as_mut().expect("open").0;
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, group, &[]);
                // A sky program that reads the pack's SSBOs (Noble's `gbuffers_textured`, the
                // sun's fallback) binds them, as the other passes do.
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                sky.geometry.bind(rp, part.textured);
                open.as_mut().expect("open").1 = i;
            }
            let rp = &mut open.as_mut().expect("open").0;
            if pass.draw {
                self.draws.bind(rp, part.stage);
            }
            rp.draw(part.range, 0..1);
        }
        drop(open);
        self.sky = Some(sky);
    }

    /// Entities, block entities, translucent entity parts and particles with the pack's
    /// programs, those drawn before deferred or (`late`) after: each run `prepare_entities` kept
    /// is one draw with its own `entityId` or `blockEntityId` and `entityColor`.
    fn draw_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        late: bool,
    ) {
        let Some(mut entities) = self.entities.take() else {
            return;
        };
        let geometry = &entities.geometry;
        for (program, pass) in entities.passes.iter_mut().enumerate() {
            let Some(pass) = pass.as_mut().filter(|_| entities.late[program] == late) else {
                continue;
            };
            let mut draws = geometry
                .draws
                .iter()
                .filter(|d| d.pack && d.program == program)
                .peekable();
            if draws.peek().is_none() {
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            {
                let mut rp = self.begin(encoder, pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.set_bind_group(1, &geometry.table_group, &[]);
                rp.set_vertex_buffer(0, geometry.vertices());
                rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                for d in draws {
                    if pass.draw {
                        rp.set_bind_group(2, &geometry.group, &[geometry.offset(d.slot)]);
                    }
                    rp.draw_indexed(0..d.quads * 6, d.base as i32, 0..1);
                }
            }
            self.mips_valid &= !pass.written;
        }
        self.entities = Some(entities);
    }

    /// Entities and block entities into the shadow map, each run with the per-draw uniforms
    /// of its gbuffers draw (2.4): opaque, so before the `shadowtex1` copy.
    fn draw_shadow_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        let Some(mut entities) = self.entities.take() else {
            return;
        };
        let geometry = &entities.geometry;
        for (program, pass) in entities.shadow.iter_mut().enumerate() {
            let Some(pass) = pass else {
                continue;
            };
            let mut draws = (geometry.draws.iter())
                .filter(|d| d.shadow && d.program == program)
                .peekable();
            if draws.peek().is_none() {
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            let mut rp = self.begin(encoder, pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            rp.set_bind_group(1, &geometry.table_group, &[]);
            rp.set_vertex_buffer(0, geometry.vertices());
            rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
            for d in draws {
                if pass.draw {
                    rp.set_bind_group(2, &geometry.group, &[geometry.offset(d.slot)]);
                }
                rp.draw_indexed(0..d.quads * 6, d.base as i32, 0..1);
            }
        }
        self.entities = Some(entities);
    }

    /// Splits `entities` (`Frame::entities`) between the pack and Horizon by its runs
    /// (`spans`): the pack's part is uploaded for this frame's draw, Horizon's goes to `rest`.
    pub fn prepare_entities(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        entities: &[super::Vertex],
        spans: &[super::pack_draw::EntitySpan],
        crumbling: [f32; 4],
        rest: &mut Vec<super::Vertex>,
    ) {
        let shadow = self.shadow_casters();
        let Some(e) = &mut self.entities else {
            rest.clear();
            rest.extend_from_slice(entities);
            return;
        };
        let drawn = e.passes.each_ref().map(Option::is_some);
        e.geometry.prepare(
            device,
            queue,
            &self.draws.layout,
            entities,
            spans,
            drawn,
            (crumbling, self.gui_size),
            shadow,
            rest,
        );
    }

    /// Which entity runs the shadow pass draws: none without it.
    pub fn shadow_casters(&self) -> super::pack_entities::Casters {
        let Some(e) = self
            .entities
            .as_ref()
            .filter(|_| self.shadow_targets.is_some())
        else {
            return Default::default();
        };
        super::pack_entities::Casters {
            entities: e.shadow[0].is_some(),
            only_player: e.only.0,
            blocks: e.shadow[1].is_some(),
            only_lit: e.only.1,
        }
    }

    /// After the frame's submission: the centre depth it copied starts its way back.
    pub fn submitted(&mut self) {
        if let Some(center) = &mut self.center_depth {
            center.submitted();
        }
    }

    /// Whether the pack draws entities or block entities itself.
    pub fn has_entities(&self) -> bool {
        self.entities.is_some()
    }

    /// The sky, when it shows, then terrain, entities and block entities.
    fn draw_opaque(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        frame: &super::Frame,
        terrain: &mut TerrainDraw<'_>,
    ) {
        if super::sky_visible(frame) {
            self.draw_sky(device, queue, encoder, size, frame);
        }
        self.draw_terrain(device, queue, encoder, size, TerrainList::Solid, terrain);
        self.draw_entities(device, queue, encoder, size, false);
        self.draw_clouds(device, queue, encoder, size);
        self.draw_hand(device, queue, encoder, size, frame);
    }

    /// The first-person hand and held items, with `gbuffers_hand` at the hand's projection.
    fn draw_hand(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        frame: &super::Frame,
    ) {
        let Some((mut pass, mut geometry)) = self.hand.take() else {
            return;
        };
        // The hand draws through `gbufferModelView`, which holds the nausea spin the world
        // turns with; the hand doesn't (as vanilla's), so its projection takes the spin out.
        let unspin = frame.bob * frame.spin.inverse() * frame.bob.inverse();
        let projection = super::pack_hand::projection(
            frame.hand_fov_degrees,
            size[0] as f32 / size[1].max(1) as f32,
            frame.depth_far,
        ) * unspin;
        geometry.prepare(device, queue, frame.hand, projection);
        if geometry.quads > 0 {
            let group = self.bind(device, queue, encoder, &mut pass, size);
            {
                let mut rp = self.begin(encoder, &pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.set_bind_group(1, &geometry.group, &[]);
                if pass.draw {
                    self.draws.bind(&mut rp, "HAND_SOLID");
                }
                rp.set_vertex_buffer(0, geometry.vertices());
                rp.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..geometry.quads * 6, 0, 0..1);
            }
            self.mips_valid &= !pass.written;
        }
        self.hand = Some((pass, geometry));
    }

    /// Whether the pack draws the first-person hand itself.
    pub fn has_hand(&self) -> bool {
        self.hand.is_some()
    }

    /// The clouds `set_clouds` gave for this frame, with `gbuffers_clouds`.
    fn draw_clouds(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        let (Some(mut pass), Some(clouds)) = (self.clouds.take(), self.cloud_draw.take()) else {
            return;
        };
        let group = self.bind(device, queue, encoder, &mut pass, size);
        {
            let mut rp = self.begin(encoder, &pass);
            rp.set_pipeline(pass.render());
            rp.set_bind_group(0, &group, &[]);
            if pass.buffers
                && let Some(g) = &self.buffers.group
            {
                rp.set_bind_group(4, g, &[]);
            }
            rp.set_bind_group(1, &clouds.group, &[]);
            if pass.draw {
                self.draws.bind(&mut rp, "CLOUDS");
            }
            rp.set_vertex_buffer(0, clouds.vertices.slice(..));
            rp.set_index_buffer(clouds.indices.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..clouds.quads * 6, 0, 0..1);
        }
        self.mips_valid &= !pass.written;
        self.clouds = Some(pass);
    }

    /// Whether the pack draws the clouds itself (or hides them).
    pub fn has_clouds(&self) -> bool {
        self.hides_clouds
    }

    /// The clouds to draw this frame (`None`: none).
    pub fn set_clouds(&mut self, clouds: Option<Clouds>) {
        self.cloud_draw = clouds.filter(|c| c.quads > 0);
    }

    /// Translucent terrain, then the entity parts drawn after deferred.
    fn draw_translucent(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        terrain: &mut TerrainDraw<'_>,
    ) {
        self.draw_terrain(
            device,
            queue,
            encoder,
            size,
            TerrainList::Translucent,
            terrain,
        );
        self.draw_entities(device, queue, encoder, size, true);
        self.draw_weather(device, queue, encoder, size);
    }

    /// The rain, then the snow, `set_weather` gave for this frame, with `gbuffers_weather`.
    fn draw_weather(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
    ) {
        let (Some(mut passes), Some(weather)) = (self.weather.take(), self.weather_draw.take())
        else {
            return;
        };
        if weather.generation != self.weather_generation {
            self.weather_generation = weather.generation;
            for (pass, sprite) in passes.iter_mut().zip(&weather.sprites) {
                pass.atlas = sprite.clone();
                pass.groups.clear();
            }
        }
        for (pass, quads) in passes.iter_mut().zip(weather.quads.clone()) {
            if quads.is_empty() {
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            {
                let mut rp = self.begin(encoder, pass);
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                if pass.draw {
                    self.draws.bind(&mut rp, "RAIN_SNOW");
                }
                rp.set_vertex_buffer(0, weather.vertices.slice(..));
                rp.set_index_buffer(weather.indices.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(quads.start * 6..quads.end * 6, 0, 0..1);
            }
            self.mips_valid &= !pass.written;
        }
        self.weather = Some(passes);
    }

    /// Whether the pack draws rain and snow itself.
    pub fn has_weather(&self) -> bool {
        self.weather.is_some()
    }

    /// The rain and snow to draw this frame (`None`: none).
    pub fn set_weather(&mut self, weather: Option<Weather>) {
        self.weather_draw = weather;
    }

    /// Regenerates the mip chain of buffer `b`'s side that programs read.
    fn mipmap(&self, encoder: &mut wgpu::CommandEncoder, b: usize) {
        let (Some(targets), Some((pipeline, _))) = (&self.targets, self.down.get(&self.formats[b]))
        else {
            return;
        };
        let Some(mips) = &targets.mips[b] else { return };
        let side = &mips[usize::from(self.main & 1 << b != 0)];
        for (attachment, group) in &side.levels {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack mipmap"),
                color_attachments: &[attach(
                    attachment,
                    wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                )],
                ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, group, &[]);
            rp.draw(0..3, 0..1);
        }
    }

    /// What the shadow camera draws of the terrain this frame, when the pack draws it.
    pub fn shadow_view(&self, frame: &super::Frame) -> Option<ShadowView> {
        if !self.has_shadow() {
            return None;
        }
        let shadow = &self.uniforms.shadow;
        Some(ShadowView {
            cull: pack_uniforms::shadow_culling(frame, shadow),
            light: pack_uniforms::shadow_light(frame, shadow),
            distance: (shadow.render_mul > 0.0).then_some(shadow.distance * shadow.render_mul),
            unculled: match self.shadow_culling {
                horizon_shaders::ShadowCulling::On => None,
                horizon_shaders::ShadowCulling::Off => Some(f32::INFINITY),
                horizon_shaders::ShadowCulling::Reversed => Some(shadow.voxel_distance),
            },
        })
    }

    /// Whether the pack draws terrain itself.
    pub fn has_terrain(&self) -> bool {
        self.terrain.is_some()
    }

    /// Whether the pack's shadow pass draws terrain (`shadow_view`).
    pub fn has_shadow(&self) -> bool {
        self.shadow_targets.is_some()
            && (self.shadow_terrain.is_some() || self.shadow_water.is_some())
    }

    /// Whether the pack draws the sky (disc, sunset, stars, void) itself.
    /// What the world's background is when the pack draws the sky: colortex0's clear colour
    /// (`None`: the fog colour, its default).
    pub fn sky_clear(&self) -> Option<[f32; 4]> {
        self.sky_clear.filter(|_| self.has_sky())
    }

    /// Whether the pack has a sky program, drawing or not: vanilla's sky is then not drawn.
    pub fn has_sky(&self) -> bool {
        self.hides_sky
    }

    /// Whether the pack has the sun and moon's program, drawing or not.
    pub fn has_celestial(&self) -> bool {
        self.hides_celestial
    }

    /// Whether the pack draws translucent terrain itself.
    pub fn has_water(&self) -> bool {
        self.water.is_some()
    }

    /// The names of the programs that run, in order.
    pub fn programs(&self) -> Vec<&str> {
        self.passes.iter().map(|p| p.name.as_str()).collect()
    }

    /// Drops every pass's cached bind groups, compute stages included: they name views that
    /// were made again.
    fn forget_groups(&mut self) {
        let sky = self
            .sky
            .iter_mut()
            .flat_map(|s| s.basic.iter_mut().chain(&mut s.textured));
        let entities = self
            .entities
            .iter_mut()
            .flat_map(|e| e.passes.iter_mut().chain(&mut e.shadow).flatten());
        let passes = self
            .passes
            .iter_mut()
            .chain(&mut self.terrain)
            .chain(&mut self.water)
            .chain(sky)
            .chain(entities)
            .chain(&mut self.clouds)
            .chain(self.weather.iter_mut().flatten())
            .chain(self.hand.iter_mut().map(|h| &mut h.0))
            .chain(&mut self.shadow_terrain)
            .chain(&mut self.shadow_water)
            .chain(&mut self.shadow_comp);
        for pass in passes {
            pass.groups.clear();
            for c in &mut pass.compute {
                c.groups.clear();
            }
        }
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if pack_images::resize_images(device, &mut self.images, size) {
            self.forget_groups();
        }
        self.buffers.resize(device, size);
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let sizes: [[u32; 2]; BUFFERS] =
            std::array::from_fn(|i| self.sizes[i].map_or(size, |s| s.resolve(size)));
        let full_chain = |size: [u32; 2]| 32 - size[0].max(size[1]).max(1).leading_zeros();
        let texture_sized = |label: &str, format, usage, levels, size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let texture = |label: &str, format, usage| texture_sized(label, format, usage, 1, size);
        let level = |t: &wgpu::Texture, level: u32| {
            t.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut mips: [Option<[Mips; 2]>; BUFFERS] = Default::default();
        let color = std::array::from_fn(|i| {
            (self.used & 1 << i != 0).then(|| {
                let label = format!("colortex{i}");
                let mipped = self.mipped & 1 << i != 0;
                let levels = if mipped { full_chain(sizes[i]) } else { 1 };
                let mut attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
                if self.storage & 1 << i != 0 {
                    attachment |= wgpu::TextureUsages::STORAGE_BINDING;
                    // Atomics on a texture need it made for them (Metal ignores them otherwise).
                    if device.features().contains(wgpu::Features::TEXTURE_ATOMIC)
                        && matches!(
                            self.formats[i],
                            wgpu::TextureFormat::R32Sint | wgpu::TextureFormat::R32Uint
                        )
                    {
                        attachment |= wgpu::TextureUsages::STORAGE_ATOMIC;
                    }
                }
                if self.copied & 1 << i != 0 {
                    attachment |= wgpu::TextureUsages::COPY_SRC;
                }
                let sides = [0, 1]
                    .map(|_| texture_sized(&label, self.formats[i], attachment, levels, sizes[i]));
                if mipped && let Some((_, layout)) = self.down.get(&self.formats[i]) {
                    mips[i] = Some(sides.each_ref().map(|t| {
                        Mips {
                            all: t.create_view(&Default::default()),
                            levels: (1..levels)
                                .map(|l| {
                                    let group =
                                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some(&label),
                                            layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &level(t, l - 1),
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.linear,
                                                    ),
                                                },
                                            ],
                                        });
                                    (level(t, l), group)
                                })
                                .collect(),
                        }
                    }));
                }
                sides.map(|t| level(&t, 0))
            })
        });
        let depth32 = wgpu::TextureFormat::Depth32Float;
        let depth_texture = texture(
            "depthtex0",
            depth32,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth1 = self.depth1.then(|| {
            let t = texture("depthtex1", depth32, wgpu::TextureUsages::COPY_DST);
            let view = t.create_view(&Default::default());
            (t, view)
        });
        let scratch = std::array::from_fn(|i| {
            (self.copied & 1 << i != 0).then(|| {
                // A copy also read as an image before a dispatch.
                let image = if self.storage & 1 << i != 0 {
                    wgpu::TextureUsages::STORAGE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                };
                texture_sized(
                    &format!("colortex{i} copy"),
                    self.formats[i],
                    wgpu::TextureUsages::COPY_DST | image,
                    1,
                    sizes[i],
                )
                .create_view(&Default::default())
            })
        });
        let sink = texture(
            "Shader pack sink",
            SINK,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .create_view(&Default::default());
        self.targets = Some(Targets {
            size,
            sink,
            sizes,
            scratch,
            color,
            depth: depth_texture.create_view(&Default::default()),
            depth_texture,
            depth1,
            mips,
        });
        self.forget_groups();
    }

    /// The shadow pass (2.1, passes 2 and 3): the shadow buffers cleared, then the `shadowcomp`
    /// programs, each drawing into the "alt" side of what it writes and flipping it after.
    fn run_shadow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &mut TerrainDraw<'_>,
        frame: &super::Frame,
    ) {
        // Opt-in only (`horizon-graphics.txt` shadowCache). Off: always redraw, drop any cache.
        if !frame.shadow_cache {
            self.shadow_cache = ShadowCache::default();
            self.draw_shadow_pass(device, queue, encoder, terrain);
            return;
        }

        // Compute the current frame's cache key before borrowing shadow_targets mutably.
        let cache_key = self.compute_shadow_cache_key(frame);

        // Entity vertices are interpolated each frame, so their shadows cannot be reused.
        let has_shadow_entities = self
            .entities
            .as_ref()
            .is_some_and(|entities| entities.geometry.draws.iter().any(|draw| draw.shadow));
        // Custom images are cleared each frame (`pack_images::clear`) before the
        // shadow pass writes them, so a pack with any redraws it too.
        let cache_hit = !has_shadow_entities
            && self.images.is_empty()
            && self.shadow_cache.key == Some(cache_key)
            && self.shadow_targets.is_some();
        if cache_hit {
            // The targets still hold this map (and `main`, the A/B side
            // `shadowcomp` left current).
            return;
        }
        self.draw_shadow_pass(device, queue, encoder, terrain);
        self.shadow_cache.key = self.shadow_targets.is_some().then_some(cache_key);
    }

    /// The shadow pass body: terrain, entities, clears, and `shadowcomp`.
    fn draw_shadow_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &mut TerrainDraw<'_>,
    ) {
        let Some(t) = &mut self.shadow_targets else {
            return;
        };
        t.begin_frame();
        let size = [t.size; 2];
        self.draw_terrain(
            device,
            queue,
            encoder,
            size,
            TerrainList::ShadowSolid,
            terrain,
        );
        self.draw_shadow_entities(device, queue, encoder, size);
        let t = self.shadow_targets.as_mut().expect("shadow pass");
        t.clear_rest(encoder);
        t.copy_depth1(encoder);
        self.draw_terrain(
            device,
            queue,
            encoder,
            size,
            TerrainList::ShadowTranslucent,
            terrain,
        );
        let mut comp = std::mem::take(&mut self.shadow_comp);
        for pass in &mut comp {
            let mut compute = std::mem::take(&mut pass.compute);
            for c in &mut compute {
                self.dispatch(device, queue, encoder, c, size);
            }
            pass.compute = compute;
            if matches!(pass.pipeline, Pipeline::Compute { .. }) {
                self.dispatch(device, queue, encoder, pass, size);
                continue;
            }
            let group = self.bind(device, queue, encoder, pass, size);
            let t = self.shadow_targets.as_ref().expect("shadow pass");
            let mut attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = Vec::new();
            for &(location, buffer) in &pass.writes {
                let l = location as usize;
                if attachments.len() <= l {
                    attachments.resize(l + 1, None);
                }
                // Overwritten whole.
                let ops = wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                };
                attachments[l] = t.color(buffer, true).and_then(|v| attach(v, ops));
            }
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&pass.name),
                    color_attachments: &attachments,
                    timestamp_writes: self.timing.as_mut().and_then(|t| t.writes(&pass.name)),
                    ..Default::default()
                });
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.draw(0..6, 0..1);
            }
            if let Some(t) = &mut self.shadow_targets {
                t.main ^= pass.flip;
            }
        }
        self.shadow_comp = comp;
    }

    /// Computes the cache key for the current frame.
    fn compute_shadow_cache_key(&self, frame: &super::Frame) -> ShadowCacheKey {
        let shadow = &self.uniforms.shadow;
        // Use the same logic as shadow_camera to compute the light direction and snapped camera.
        // Use the public shadow_light function which returns the normalized light direction.
        let light = super::pack_uniforms::shadow_light(frame, shadow);
        let light = light.as_dvec3();
        let up = if light.z.abs() > 0.99 {
            DVec3::Y
        } else {
            DVec3::Z
        };
        let rotation = DMat4::look_to_rh(DVec3::ZERO, -light, up);
        let at = rotation.transform_vector3(frame.camera);
        let step = f64::from(shadow.interval);
        let snapped_camera = if step > 0.0 {
            Some((at / step).floor() * step)
        } else {
            Some(at)
        };

        // Hash the shadow settings that affect the shadow map.
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        shadow.resolution.hash(&mut hasher);
        shadow.distance.to_bits().hash(&mut hasher);
        shadow.render_mul.to_bits().hash(&mut hasher);
        shadow.interval.to_bits().hash(&mut hasher);
        shadow.sun_path_rotation.to_bits().hash(&mut hasher);
        shadow.fov.map(|f| f.to_bits()).hash(&mut hasher);
        shadow.near.to_bits().hash(&mut hasher);
        shadow.far.to_bits().hash(&mut hasher);
        for hf in &shadow.hardware_filtering {
            hf.hash(&mut hasher);
        }
        for color in &shadow.colors {
            format!("{:?}", color.format).hash(&mut hasher);
            color.clear.hash(&mut hasher);
            if let Some(cc) = color.clear_color {
                for c in cc {
                    c.to_bits().hash(&mut hasher);
                }
            }
            color.mipmap.hash(&mut hasher);
        }
        let shadow_settings_hash = hasher.finish();
        let shadow_uniforms_hash = self.shadow_uniforms_hash();

        ShadowCacheKey {
            sun_angle: frame.sky.sun_angle,
            moon_angle: frame.sky.moon_angle,
            snapped_camera,
            shadow_settings_hash,
            shadow_scene_revision: frame.shadow_scene_revision,
            ticks: frame.ticks,
            day_time: frame.pack.day_time,
            shadow_uniforms_hash,
        }
    }

    fn shadow_uniforms_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let passes = self
            .shadow_terrain
            .iter()
            .chain(self.shadow_water.iter())
            .chain(self.shadow_comp.iter())
            .chain(
                self.entities
                    .iter()
                    .flat_map(|entities| entities.shadow.iter().flatten()),
            );
        for pass in passes {
            pass.name.hash(&mut hasher);
            for (index, ty, offset) in &pass.uniforms {
                index.hash(&mut hasher);
                ty.hash(&mut hasher);
                offset.hash(&mut hasher);
                let Some(value) = index.and_then(|index| self.uniforms.values.get(index)) else {
                    continue;
                };
                match value {
                    pack_uniforms::Value::F(values) => {
                        0u8.hash(&mut hasher);
                        for value in values {
                            value.to_bits().hash(&mut hasher);
                        }
                    }
                    pack_uniforms::Value::I(values) => {
                        1u8.hash(&mut hasher);
                        values.hash(&mut hasher);
                    }
                }
            }
        }
        hasher.finish()
    }

    /// Runs the pack over the world drawn into `world` (colour) and `world_depth`, then draws
    /// the result onto `screen`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        screen: &wgpu::TextureView,
        frame: &super::Frame,
        inputs: &PackInputs,
        size: [u32; 2],
        window: [u32; 2],
        pages: &[(u32, wgpu::Buffer)],
        terrain: &mut TerrainDraw<'_>,
    ) {
        self.resize(device, size);
        if let Some((layout, ids)) = &self.block_ids {
            // A page that grew has a new buffer.
            self.page_groups
                .retain(|p, (b, _)| pages.iter().any(|(q, c)| q == p && c == b));
            for (page, buffer) in pages {
                self.page_groups.entry(*page).or_insert_with(|| {
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Shader pack terrain"),
                        layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: ids.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: buffer.as_entire_binding(),
                            },
                        ],
                    });
                    (buffer.clone(), group)
                });
            }
        }
        if let Some(t) = &mut self.timing {
            t.begin(device, queue);
        }
        self.uniforms.center_depth_raw = self
            .center_depth
            .get_or_insert_with(|| super::pack_center_depth::CenterDepth::new(device))
            .collect(device);
        self.uniforms.update(frame, inputs, size);
        // Until the camera's biome is known, `smooth` waits for the world's values.
        if !inputs.biome_known {
            self.custom.restart_smoothing();
        }
        self.uniforms.eval_custom(&mut self.custom);
        if self.targets.is_none() {
            return;
        }
        // Images start the frame cleared, before the shadow pass writes them.
        if let Some(dump) = &mut self.dump {
            dump.read(device);
            dump.copy(device, encoder, &self.images);
        }
        pack_images::clear(
            device,
            encoder,
            &self.images,
            self.gpu.clear,
            &mut self.zeros,
        );
        self.run_shadow(device, queue, encoder, terrain, frame);
        let Some(targets) = &self.targets else { return };
        let main = |m: u16, i: usize| usize::from(m & 1 << i != 0);
        let view = |m: u16, i: usize, alt: bool| {
            targets.color[i]
                .as_ref()
                .map(|t| &t[main(m, i) ^ usize::from(alt)])
        };

        if !self.shadow_cleared {
            self.shadow_cleared = true;
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack shadow stand-in"),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow.0,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        let clear_op = |b: usize| {
            let c = self.clears[b].unwrap_or_default();
            wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: c[0].into(),
                    g: c[1].into(),
                    b: c[2].into(),
                    a: c[3].into(),
                }),
                store: wgpu::StoreOp::Store,
            }
        };
        // Overwritten whole: clearing lets a tiled GPU skip loading the old content.
        let overwrite = wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        };
        // Attachments of a pass share a size.
        let mut by_size: Vec<([u32; 2], Vec<usize>)> = Vec::new();
        for &b in &self.other_clears {
            let at = targets.sizes[b];
            match by_size.iter_mut().find(|(s, l)| *s == at && l.len() < 8) {
                Some((_, l)) => l.push(b),
                None => by_size.push((at, vec![b])),
            }
        }
        for (_, chunk) in &by_size {
            let attachments: Vec<_> = chunk
                .iter()
                .map(|&b| attach(view(self.main, b, false).expect("used buffer"), clear_op(b)))
                .collect();
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack clear"),
                color_attachments: &attachments,
                timestamp_writes: self.timing.as_mut().and_then(|t| t.writes("clear")),
                ..Default::default()
            });
        }

        // The world into colortex0 and depthtex0, flipped to GL's rows, and the clears.
        if self.blit_group.as_ref().is_none_or(|g| g.0 != size) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack world"),
                layout: &self.blit.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(world),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(world_depth),
                    },
                ],
            });
            self.blit_group = Some((size, group));
        }
        {
            let mut attachments = vec![attach(
                view(self.main, 0, false).expect("colortex0"),
                overwrite,
            )];
            for &b in &self.blit_clears {
                attachments.push(attach(
                    view(self.main, b, false).expect("used buffer"),
                    clear_op(b),
                ));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack world"),
                color_attachments: &attachments,
                timestamp_writes: self.timing.as_mut().and_then(|t| t.writes("world")),
                depth_stencil_attachment: self.blit_depth.then_some(
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &targets.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    },
                ),
                ..Default::default()
            });
            pass.set_pipeline(&self.blit.0);
            pass.set_bind_group(0, self.blit_group.as_ref().map(|g| &g.1), &[]);
            pass.draw(0..3, 0..1);
        }

        // The world and the clears just rewrote the buffers' first levels.
        self.mips_valid = 0;
        let mut passes = std::mem::take(&mut self.passes);
        let mut seen = (false, false);
        let mut shown = false;
        let mut terrain_drawn = self.terrain.is_none()
            && self.sky.is_none()
            && !self.entities.as_ref().is_some_and(Entities::drawn)
            && self.clouds.is_none()
            && self.hand.is_none();
        let late_entities = self.entities.as_ref().is_some_and(|e| {
            e.passes
                .iter()
                .zip(e.late)
                .any(|(p, late)| p.is_some() && late)
        });
        let mut water_drawn = self.water.is_none() && self.weather.is_none() && !late_entities;
        for pass in &mut passes {
            if !terrain_drawn && pass.group != Group::Prepare {
                terrain_drawn = true;
                self.draw_opaque(device, queue, encoder, size, frame, terrain);
            }
            if !water_drawn && matches!(pass.group, Group::Composite | Group::Final) {
                water_drawn = true;
                self.draw_translucent(device, queue, encoder, size, terrain);
            }
            if pass.group == Group::Deferred && !seen.0 {
                seen.0 = true;
                self.main ^= self.deferred_pre;
            }
            if pass.group == Group::Composite && !seen.1 {
                seen.1 = true;
                self.main ^= self.composite_pre;
            }
            let mut compute = std::mem::take(&mut pass.compute);
            for c in &mut compute {
                self.dispatch(device, queue, encoder, c, size);
            }
            pass.compute = compute;
            if matches!(pass.pipeline, Pipeline::Compute { .. }) {
                self.dispatch(device, queue, encoder, pass, size);
                continue;
            }
            let stale = pass.mipmaps & self.mipped & !self.mips_valid;
            for b in (0..BUFFERS).filter(|b| stale & 1 << b != 0) {
                self.mipmap(encoder, b);
            }
            self.mips_valid |= stale;
            let final_pass = pass.group == Group::Final;
            // A pass writing buffers of a size of their own draws at that size (they share it).
            let out = match pass.writes.first() {
                _ if final_pass => window,
                Some(&(_, b)) => self.targets.as_ref().map_or(size, |t| t.sizes[b]),
                None => size,
            };
            let group = self.bind(device, queue, encoder, pass, out);
            let targets = self.targets.as_ref().expect("resized");
            let view = |i: usize| {
                targets.color[i]
                    .as_ref()
                    .map(|t| &t[usize::from(self.main & 1 << i == 0)])
            };
            let mut attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = Vec::new();
            if final_pass {
                attachments.push(attach(screen, overwrite));
                shown = true;
            } else if pass.sink {
                attachments.push(attach(&targets.sink, overwrite));
            } else {
                for &(location, buffer) in &pass.writes {
                    let l = location as usize;
                    if attachments.len() <= l {
                        attachments.resize(l + 1, None);
                    }
                    attachments[l] = view(buffer).and_then(|v| attach(v, overwrite));
                }
            }
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&pass.name),
                    color_attachments: &attachments,
                    timestamp_writes: self.timing.as_mut().and_then(|t| t.writes(&pass.name)),
                    ..Default::default()
                });
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, &group, &[]);
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                rp.draw(0..6, 0..1);
            }
            self.main ^= pass.flip;
            self.mips_valid &= !pass.written;
        }
        self.passes = passes;
        if !terrain_drawn {
            self.draw_opaque(device, queue, encoder, size, frame, terrain);
        }
        if !water_drawn {
            self.draw_translucent(device, queue, encoder, size, terrain);
        }
        let targets = self.targets.as_ref().expect("resized");
        // depthtex0 as the frame leaves it, for next frames' `centerDepthSmooth`.
        if let Some(center) = &mut self.center_depth {
            center.copy(device, encoder, &targets.depth_texture);
        }
        let view = |m: u16, i: usize, alt: bool| {
            targets.color[i]
                .as_ref()
                .map(|t| &t[main(m, i) ^ usize::from(alt)])
        };
        if !shown {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack colortex0"),
                layout: &self.copy.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            view(self.main, 0, false).expect("colortex0"),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear),
                    },
                ],
            });
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack colortex0"),
                color_attachments: &[attach(screen, overwrite)],
                ..Default::default()
            });
            rp.set_pipeline(&self.copy.0);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat as T;

    #[test]
    fn shadow_cache_key_tracks_world_and_time_changes() {
        let key = ShadowCacheKey {
            sun_angle: 0.0,
            moon_angle: 180.0,
            snapped_camera: Some(DVec3::ZERO),
            shadow_settings_hash: 1,
            shadow_scene_revision: (2, 3),
            ticks: 4,
            day_time: 5,
            shadow_uniforms_hash: 6,
        };

        assert_ne!(
            key,
            ShadowCacheKey {
                shadow_scene_revision: (8, 3),
                ..key
            }
        );
        assert_ne!(key, ShadowCacheKey { day_time: 9, ..key });
        assert_ne!(
            key,
            ShadowCacheKey {
                shadow_uniforms_hash: 10,
                ..key
            }
        );
    }

    /// Two textures with a sampler each become two textures sharing one sampler variable.
    #[test]
    fn samplers_of_a_kind_share_one_variable() {
        let mut module = naga::front::wgsl::parse_str(
            "@group(0) @binding(1) var a: texture_2d<f32>;\n\
             @group(0) @binding(2) var sa: sampler;\n\
             @group(0) @binding(3) var b: texture_2d<f32>;\n\
             @group(0) @binding(4) var sb: sampler;\n\
             @fragment fn main() -> @location(0) vec4<f32> {\n\
             return textureSample(a, sa, vec2(0.5)) + textureSample(b, sb, vec2(0.5));\n}\n",
        )
        .unwrap();
        share_samplers(&mut module, &HashMap::from([(2, 2), (4, 2)]));
        let info = naga::valid::Validator::new(Default::default(), Default::default())
            .validate(&module)
            .unwrap();
        let used: Vec<u32> = module
            .global_variables
            .iter()
            .filter(|(h, _)| !info.get_entry_point(0)[*h].is_empty())
            .filter_map(|(_, g)| g.binding.as_ref().map(|b| b.binding))
            .collect();
        assert_eq!(used, [1, 2, 3]);
    }

    /// #3525: naga's SPIR-V writer panicked on Photon's gbuffers programs as the pipeline was
    /// made. Such a panic skips the pass, as a validation error does, and the device goes on.
    #[test]
    fn a_shader_compiler_panic_skips_the_pass() {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            return;
        };
        let failed = checked(&device, || -> wgpu::Buffer {
            panic!("internal error: entered unreachable code: Expression [89] is not cached!")
        });
        let error = failed.expect_err("the panic comes back as an error");
        assert!(error.contains("Expression [89] is not cached!"), "{error}");
        let buffer = |usage| {
            checked(&device, || {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 16,
                    usage,
                    mapped_at_creation: false,
                })
            })
        };
        // The device still makes things, and its errors still come back.
        assert!(buffer(wgpu::BufferUsages::UNIFORM).is_ok());
        assert!(buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::MAP_WRITE).is_err());
    }

    #[test]
    fn pack_formats_map_to_renderable_ones() {
        let f = |name| texture_format(Format::parse(name).unwrap(), wgpu::Features::empty());
        assert_eq!(f("RGBA"), T::Rgba8Unorm);
        assert_eq!(f("RGB8"), T::Rgba8Unorm, "no three-channel formats in wgpu");
        assert_eq!(f("RG16F"), T::Rg16Float);
        assert_eq!(f("R32F"), T::R32Float);
        assert_eq!(f("RGB10_A2"), T::Rgb10a2Unorm);
        assert_eq!(f("R32UI"), T::R32Uint);
        assert_eq!(f("RGBA16"), T::Rgba16Float, "16-bit norm needs a feature");
        assert_eq!(f("RGBA8_SNORM"), T::Rgba16Float, "snorm is not renderable");
        assert_eq!(f("R11F_G11F_B10F"), T::Rgba16Float);
        let all = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
            | wgpu::Features::RG11B10UFLOAT_RENDERABLE;
        let g = |name| texture_format(Format::parse(name).unwrap(), all);
        assert_eq!(g("RGBA16"), T::Rgba16Unorm);
        assert_eq!(g("R11F_G11F_B10F"), T::Rg11b10Ufloat);
    }

    #[test]
    fn samplers_read_custom_textures_of_their_type() {
        use wgpu::TextureSampleType as T;
        let float = T::Float { filterable: true };
        assert!(fits("sampler2D", float));
        assert!(fits("sampler3D", float));
        assert!(fits("isampler2D", T::Sint));
        assert!(fits("usampler1D", T::Uint));
        assert!(!fits("sampler2D", T::Uint));
        assert!(!fits("isampler2D", float));
        assert!(!fits("sampler2DShadow", float));
    }

    #[test]
    fn sampler_names() {
        let main = |n| source(n, false, false, false);
        assert!(main("gaux1") == Source::Color(4));
        assert!(main("gdepthtex") == Source::Depth);
        assert!(main("depthtex2") == Source::Depth1);
        assert!(main("noisetex") == Source::Noise);
        // Without the shadow pass, the shadow buffers are stand-ins.
        assert!(main("shadowcolor0") == Source::Dummy);
        assert!(main("shadowtex0") == Source::Dummy);
        // With it: shadowcolor n is buffer n of the shadow programs, `shadow` is shadowtex1
        // beside `watershadow` (6.3), and colortex is not theirs.
        assert!(source("shadowcolor", false, true, false) == Source::ShadowColor(0));
        assert!(source("shadowcolor1", true, true, false) == Source::Color(1));
        assert!(source("colortex1", true, true, false) == Source::Dummy);
        assert!(source("shadow", false, true, false) == Source::ShadowDepth(0, false));
        assert!(source("shadow", false, true, true) == Source::ShadowDepth(1, false));
        assert!(source("watershadow", false, true, true) == Source::ShadowDepth(0, false));
        assert!(source("shadowtex1HW", false, true, false) == Source::ShadowDepth(1, true));
    }
}
