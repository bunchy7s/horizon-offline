//! Vanilla's post-effect chains (`PostChainConfig`, `PostChain`, `PostPass`,
//! `GameRenderer.preparePostEffects` / `applyPostEffects`): a `post_effect/*.json`
//! names internal targets and passes (a vertex and a fragment shader, inputs read
//! from targets, the target written, std140 uniform blocks) that run over the
//! frame drawn so far, before the HUD.
//!
//! The JSON is read at run time from the active resource root, so a resource pack
//! can replace or add chains. So are the shaders the passes name: their GLSL
//! (`shaders/<path>.vsh` / `.fsh`, vanilla's or a pack's own) is translated to naga
//! by `post_glsl` and bound as vanilla binds it (`SamplerInfo`, `Globals`, the pass's
//! own uniform blocks, a sampler per input: a target's colour or depth, or a
//! `textures/effect/` image). Horizon's own WGSL stand-ins (`post_chain.wgsl`) for the
//! vanilla `core/screenquad`, `post/rotscale`, `post/invert`, `post/blit`, `post/bits`,
//! `post/color_convolve`, `post/box_blur` and `post/spiderclip` are only the fallback
//! for a pass whose GLSL is missing or does not translate. A chain that cannot load
//! (an unknown shader without a stand-in, a target the frame has no equivalent of)
//! fails like a chain that does not compile (`GameRenderer.failedPostEffects`), once,
//! and is skipped.
//!
//! Every target keeps GL's rows (row 0 is the bottom edge): the frame is flipped into
//! the chain's `main` and back, and each pass draws with clip y negated.
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `PostChain.MAIN_TARGET_ID`.
const MAIN: &str = "minecraft:main";
/// `PostPass`'s uniform buffers are 128 bytes.
const BLOCK_BYTES: usize = 128;
/// Our uniform buffers are bigger than the blocks they feed, so a shader that declares
/// more than vanilla writes still binds.
const UNIFORM_BYTES: u64 = 256;

/// `ResourceLocation.withDefaultNamespace` on an id read from a chain.
fn normalise(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// One uniform of a block (`UniformValue`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uniform {
    Int(i32),
    IVec3([i32; 3]),
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Matrix([f32; 16]),
}

impl Uniform {
    /// std140 (alignment, size) in bytes: `Std140SizeCalculator`.
    fn layout(&self) -> (usize, usize) {
        match self {
            Uniform::Int(_) | Uniform::Float(_) => (4, 4),
            Uniform::Vec2(_) => (8, 8),
            Uniform::Vec3(_) | Uniform::IVec3(_) => (16, 12),
            Uniform::Vec4(_) => (16, 16),
            Uniform::Matrix(_) => (16, 64),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Uniform::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
            Uniform::IVec3(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Float(v) => out.extend_from_slice(&v.to_le_bytes()),
            Uniform::Vec2(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Vec3(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Vec4(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Matrix(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
        }
    }
}

/// A uniform block in std140 order, as `PostPass` builds it with `Std140Builder`.
pub fn pack_block(values: &[Uniform]) -> Result<[u8; BLOCK_BYTES], String> {
    let mut bytes = Vec::new();
    for value in values {
        let (align, _) = value.layout();
        while bytes.len() % align != 0 {
            bytes.push(0);
        }
        value.write(&mut bytes);
    }
    if bytes.len() > BLOCK_BYTES {
        return Err(format!("a uniform block of {} bytes", bytes.len()));
    }
    let mut block = [0u8; BLOCK_BYTES];
    block[..bytes.len()].copy_from_slice(&bytes);
    Ok(block)
}

/// `PostChainConfig.InternalTarget`.
#[derive(Clone, Debug, PartialEq)]
pub struct InternalTarget {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub persistent: bool,
    /// `clear_color` as straight RGBA.
    pub clear: [f32; 4],
}

/// What an input reads: `PostChainConfig.TargetInput` or `TextureInput`.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A target's colour, or its depth buffer (`use_depth_buffer`).
    Target { id: String, depth: bool },
    /// `textures/effect/<location>.png`.
    Texture { location: String },
}

/// `PostChainConfig.Input`.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub sampler: String,
    pub source: Source,
    pub bilinear: bool,
}

/// `PostChainConfig.Pass`.
#[derive(Clone, Debug, PartialEq)]
pub struct PassConfig {
    pub vertex: String,
    pub fragment: String,
    pub inputs: Vec<Input>,
    pub output: String,
    pub uniforms: BTreeMap<String, Vec<Uniform>>,
}

/// `PostChainConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub targets: BTreeMap<String, InternalTarget>,
    pub passes: Vec<PassConfig>,
}

fn floats<const N: usize>(v: &Value) -> Result<[f32; N], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == N)
        .ok_or("a vector of the wrong length")?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().ok_or("a vector entry that is not a number")? as f32;
    }
    Ok(out)
}

fn uniform(v: &Value) -> Result<Uniform, String> {
    let kind = v["type"].as_str().ok_or("a uniform without a type")?;
    let value = &v["value"];
    Ok(match kind {
        "int" => Uniform::Int(value.as_i64().ok_or("a bad int")? as i32),
        "float" => Uniform::Float(value.as_f64().ok_or("a bad float")? as f32),
        "vec2" => Uniform::Vec2(floats(value)?),
        "vec3" => Uniform::Vec3(floats(value)?),
        "vec4" => Uniform::Vec4(floats(value)?),
        "ivec3" => {
            let a = value
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or("a bad ivec3")?;
            let mut out = [0; 3];
            for (o, x) in out.iter_mut().zip(a) {
                *o = x.as_i64().ok_or("a bad ivec3")? as i32;
            }
            Uniform::IVec3(out)
        }
        "matrix4x4" => {
            let flat: Vec<f32> = match value.as_array() {
                Some(rows) if rows.first().is_some_and(Value::is_array) => rows
                    .iter()
                    .flat_map(|r| r.as_array().into_iter().flatten())
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect(),
                Some(flat) => flat
                    .iter()
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect(),
                None => Vec::new(),
            };
            Uniform::Matrix(flat.try_into().map_err(|_| "a bad matrix4x4")?)
        }
        other => return Err(format!("unknown uniform type {other}")),
    })
}

impl Config {
    /// `PostChainConfig.CODEC` over the file's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut targets = BTreeMap::new();
        if let Some(map) = json.get("targets").and_then(Value::as_object) {
            for (id, t) in map {
                let size = |key: &str| -> Result<Option<u32>, String> {
                    match t.get(key) {
                        None => Ok(None),
                        Some(v) => match v.as_u64() {
                            Some(n) if n > 0 => Ok(Some(n as u32)),
                            _ => Err(format!("target {id}: a bad {key}")),
                        },
                    }
                };
                let argb = t.get("clear_color").and_then(Value::as_u64).unwrap_or(0) as u32;
                let [a, r, g, b] = [24, 16, 8, 0].map(|s| ((argb >> s) & 255) as f32 / 255.0);
                targets.insert(
                    normalise(id),
                    InternalTarget {
                        width: size("width")?,
                        height: size("height")?,
                        persistent: t
                            .get("persistent")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        clear: [r, g, b, a],
                    },
                );
            }
        }
        let mut passes = Vec::new();
        for (n, p) in json
            .get("passes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let text = |key: &str| -> Result<String, String> {
                p.get(key)
                    .and_then(Value::as_str)
                    .map(normalise)
                    .ok_or(format!("pass {n}: no {key}"))
            };
            let mut inputs: Vec<Input> = Vec::new();
            for i in p
                .get("inputs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let sampler = i["sampler_name"]
                    .as_str()
                    .ok_or(format!("pass {n}: an input without sampler_name"))?;
                if inputs.iter().any(|x| x.sampler == sampler) {
                    return Err(format!("Encountered repeated sampler name: {sampler}"));
                }
                // Codec.xor(TextureInput, TargetInput): exactly one of them parses.
                let source = match (i.get("location"), i.get("target")) {
                    (Some(location), None) => {
                        for key in ["width", "height"] {
                            if !i.get(key).and_then(Value::as_u64).is_some_and(|v| v > 0) {
                                return Err(format!("pass {n}: a texture input needs a {key}"));
                            }
                        }
                        Source::Texture {
                            location: normalise(
                                location
                                    .as_str()
                                    .ok_or(format!("pass {n}: a bad location"))?,
                            ),
                        }
                    }
                    (None, Some(target)) => Source::Target {
                        id: normalise(target.as_str().ok_or(format!("pass {n}: a bad target"))?),
                        depth: i
                            .get("use_depth_buffer")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                    _ => return Err(format!("pass {n}: an input needs a location or a target")),
                };
                inputs.push(Input {
                    sampler: sampler.to_owned(),
                    source,
                    bilinear: i.get("bilinear").and_then(Value::as_bool).unwrap_or(false),
                });
            }
            let mut uniforms = BTreeMap::new();
            if let Some(map) = p.get("uniforms").and_then(Value::as_object) {
                for (block, list) in map {
                    let list = list
                        .as_array()
                        .ok_or(format!("pass {n}: a bad uniform block"))?;
                    uniforms.insert(
                        block.clone(),
                        list.iter().map(uniform).collect::<Result<Vec<_>, _>>()?,
                    );
                }
            }
            passes.push(PassConfig {
                vertex: text("vertex_shader")?,
                fragment: text("fragment_shader")?,
                inputs,
                output: text("output")?,
                uniforms,
            });
        }
        Ok(Config { targets, passes })
    }

    /// `PostChain.getReferencedExternalTargets`.
    pub fn external_targets(&self) -> HashSet<&str> {
        let mut out = HashSet::new();
        for p in &self.passes {
            let targets = p.inputs.iter().filter_map(|i| match &i.source {
                Source::Target { id, .. } => Some(id),
                Source::Texture { .. } => None,
            });
            for id in targets.chain([&p.output]) {
                if !self.targets.contains_key(id) {
                    out.insert(id.as_str());
                }
            }
        }
        out
    }
}

/// A stand-in for a vanilla vertex shader: its entry point and uniform block.
const VERTEX: [(&str, &str, &str); 2] = [
    ("minecraft:core/screenquad", "vs_quad_gl", ""),
    ("minecraft:post/rotscale", "vs_rotscale", "RotScaleConfig"),
];
// The stand-ins are the fallback for a pass whose GLSL is missing or does not translate.
/// A stand-in for a vanilla fragment shader: entry point, uniform block and the
/// samplers it reads (`In`, then `Blur`).
const FRAGMENT: [(&str, &str, &str, usize); 6] = [
    ("minecraft:post/invert", "fs_invert", "InvertConfig", 1),
    ("minecraft:post/blit", "fs_blit", "BlitConfig", 1),
    ("minecraft:post/bits", "fs_bits", "BitsConfig", 1),
    (
        "minecraft:post/color_convolve",
        "fs_color_convolve",
        "ColorConfig",
        1,
    ),
    ("minecraft:post/box_blur", "fs_box_blur", "BlurConfig", 1),
    (
        "minecraft:post/spiderclip",
        "fs_spiderclip",
        "SpiderConfig",
        2,
    ),
];
const SAMPLERS: [&str; 2] = ["In", "Blur"];

/// A pass input resolved against the chain's targets.
#[derive(Clone, Copy)]
enum Src {
    Target {
        index: usize,
        depth: bool,
    },
    /// `Chain::images`.
    Image(usize),
}

#[derive(Clone, Copy)]
struct InputRef {
    src: Src,
    bilinear: bool,
}

/// What a translated shader's uniform block is bound to.
enum Block {
    /// `SamplerInfo`: `OutSize`, then each input's size.
    Info,
    /// `Globals` (`GlobalSettingsUniform`).
    Globals,
    /// One of the pass's `uniforms` groups.
    Custom(wgpu::Buffer),
}

/// A pass running the resources' own GLSL, translated by `post_glsl`.
struct Translated {
    layout: wgpu::BindGroupLayout,
    vertex: wgpu::ShaderModule,
    fragment: wgpu::ShaderModule,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    /// Binding and what it holds.
    blocks: Vec<(u32, Block)>,
    /// A sampler's texture binding (the sampler object follows) and the input it reads.
    samplers: Vec<(u32, usize)>,
}

impl Translated {
    fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.pipelines.contains_key(&format) {
            return;
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Post effect"),
            bind_group_layouts: &[Some(&self.layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Post effect pass"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.vertex,
                entry_point: Some("main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.fragment,
                entry_point: Some("main"),
                // ColorTargetState.DEFAULT: no blending.
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(format, pipeline);
    }
}

struct Pass {
    /// Every input in the chain's order (translated passes).
    refs: Vec<InputRef>,
    translated: Option<Translated>,
    vs: &'static str,
    fs: &'static str,
    /// Per sampler slot: the target read and its filter.
    inputs: [(usize, bool); 2],
    output: usize,
    info: wgpu::Buffer,
    vs_block: wgpu::Buffer,
    fs_block: wgpu::Buffer,
}

/// A loaded chain (`PostChain`): its targets are 0 = the frame, then the internal
/// ones in id order.
struct Chain {
    targets: Vec<(String, InternalTarget)>,
    passes: Vec<Pass>,
    /// Textures for `targets[1..]`, with the size they were made at.
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// A depth buffer per target a pass reads the depth of (`DEFAULT_DEPTH`, cleared to 1;
    /// the frame's own when it has one).
    depths: Vec<Option<(wgpu::Texture, wgpu::TextureView)>>,
    /// The `textures/effect/` images the passes read, with their size.
    images: Vec<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    size: [u32; 2],
}

/// What a frame asks for.
pub struct Request<'a> {
    /// `GameRenderer.requestedPostEffects`, in order.
    pub effects: &'a [String],
    /// The active resource root (`assets/<namespace>/post_effect/<path>.json`).
    pub root: &'a Path,
    /// `menuBackgroundBlurriness`, the radius of a `box_blur` pass with `Radius` 0.
    pub menu_blur_radius: f32,
    /// What the `Globals` block holds besides the screen size and the blur radius.
    pub globals: Globals,
}

/// `GlobalSettingsUniform.update`'s other values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Globals {
    pub camera_block_pos: [i32; 3],
    pub glint_alpha: f32,
    /// The camera's offset inside its block.
    pub camera_offset: [f32; 3],
    /// `(gameTime % 24000 + partialTick) / 24000`.
    pub game_time: f32,
    pub use_rgss: bool,
}

impl Globals {
    /// The std140 block: `ivec3, float, vec3, float, vec2, int, int`.
    fn bytes(&self, size: [u32; 2], menu_blur_radius: i32) -> [u8; 48] {
        let mut out = Vec::with_capacity(48);
        for v in self.camera_block_pos {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.glint_alpha.to_le_bytes());
        for v in self.camera_offset {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.game_time.to_le_bytes());
        out.extend_from_slice(&(size[0] as f32).to_le_bytes());
        out.extend_from_slice(&(size[1] as f32).to_le_bytes());
        out.extend_from_slice(&menu_blur_radius.to_le_bytes());
        out.extend_from_slice(&i32::from(self.use_rgss).to_le_bytes());
        out.try_into().expect("48 bytes")
    }
}

impl<'a> Request<'a> {
    /// No chains.
    pub fn none() -> Self {
        Request {
            effects: &[],
            root: Path::new(""),
            menu_blur_radius: 0.0,
            globals: Globals::default(),
        }
    }
}

/// Every chain, the pipelines and the frame copy they work on.
pub struct PostEffects {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipelines: HashMap<(&'static str, &'static str, wgpu::TextureFormat), wgpu::RenderPipeline>,
    nearest: wgpu::Sampler,
    bilinear: wgpu::Sampler,
    /// `ShaderManager`'s chains by id; `None` when the file is missing.
    chains: HashMap<String, Option<Chain>>,
    /// `GameRenderer.failedPostEffects`.
    failed: HashSet<String>,
    root: PathBuf,
    /// The frame as the chains see `minecraft:main`.
    work: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The frame as copied, before it is flipped into `work`.
    staging: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The flip between the frame and GL's rows.
    blit: Option<Pass>,
    /// The `Globals` block.
    globals: wgpu::Buffer,
}

fn resource_path(root: &Path, id: &str) -> PathBuf {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    root.join("assets")
        .join(ns)
        .join("post_effect")
        .join(format!("{path}.json"))
}

impl PostEffects {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Post effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_chain.wgsl").into()),
        });
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Post effect bindings"),
            entries: &[
                uniform(0),
                uniform(1),
                uniform(2),
                texture(3),
                sampler(4),
                texture(5),
                sampler(6),
            ],
        });
        // SamplerCache.getClampToEdge.
        let make = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Post effect"),
                mag_filter: filter,
                min_filter: filter,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            })
        };
        Self {
            layout,
            shader,
            pipelines: HashMap::new(),
            nearest: make(wgpu::FilterMode::Nearest),
            bilinear: make(wgpu::FilterMode::Linear),
            chains: HashMap::new(),
            failed: HashSet::new(),
            root: PathBuf::new(),
            work: None,
            staging: None,
            blit: None,
            globals: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Post effect globals"),
                size: UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// `GameRenderer.reloadPostEffects`-alike: forget chains loaded from another
    /// root, and the failures with them (`shouldResetFailedPostEffects`).
    fn use_root(&mut self, root: &Path) {
        if self.root != root {
            self.root = root.to_owned();
            self.chains.clear();
            self.failed.clear();
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        key: (&'static str, &'static str),
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let layout = &self.layout;
        let shader = &self.shader;
        self.pipelines
            .entry((key.0, key.1, format))
            .or_insert_with(|| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Post effect"),
                        bind_group_layouts: &[Some(layout)],
                        ..Default::default()
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Post effect pass"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some(key.0),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some(key.1),
                        // ColorTargetState.DEFAULT: no blending.
                        targets: &[Some(format.into())],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
    }

    /// `PostChain.load` for the file of `id`: `Ok(None)` when it does not exist.
    fn load(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: &str,
    ) -> Result<Option<Chain>, String> {
        let Ok(text) = std::fs::read_to_string(resource_path(&self.root, id)) else {
            return Ok(None);
        };
        let config = Config::parse(&text)?;
        // isPostEffectValid / LevelTargetBundle.MAIN_TARGETS: the frame is the one
        // external target Horizon has.
        if let Some(bad) = config.external_targets().into_iter().find(|t| *t != MAIN) {
            return Err(format!(
                "Referenced external targets are not available in this context: [{bad}]"
            ));
        }
        let targets: Vec<(String, InternalTarget)> = std::iter::once((
            MAIN.to_owned(),
            InternalTarget {
                width: None,
                height: None,
                persistent: true,
                clear: [0.0; 4],
            },
        ))
        .chain(config.targets.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
        let index = |name: &str| targets.iter().position(|(t, _)| t == name);
        let mut images = Vec::new();
        let mut passes = Vec::new();
        for (n, p) in config.passes.iter().enumerate() {
            let output = index(&p.output).ok_or("a target is missing")?;
            let pass = match self.translated_pass(device, queue, n, p, output, &index, &mut images)
            {
                Ok(pass) => pass,
                Err(e) => match self.standin_pass(device, queue, n, p, output, &index) {
                    Ok(pass) => {
                        log::debug!("post effect {id} pass {n} uses the built-in shaders: {e}");
                        pass
                    }
                    Err(_) => return Err(e),
                },
            };
            passes.push(pass);
        }
        Ok(Some(Chain {
            targets,
            passes,
            textures: Vec::new(),
            depths: Vec::new(),
            images,
            size: [0, 0],
        }))
    }

    /// A pass over the resources' own GLSL (`PostChain.createPass`).
    #[allow(clippy::too_many_arguments)]
    fn translated_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        n: usize,
        p: &PassConfig,
        output: usize,
        index: &dyn Fn(&str) -> Option<usize>,
        images: &mut Vec<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    ) -> Result<Pass, String> {
        let program = crate::post_glsl::translate(&self.root, &p.vertex, &p.fragment)?;
        let mut refs = Vec::new();
        for input in &p.inputs {
            let src = match &input.source {
                Source::Target { id, depth } => {
                    let target = index(id).ok_or("a target is missing")?;
                    if target == output && !depth {
                        return Err(format!("pass {n}: reads the target it writes"));
                    }
                    Src::Target {
                        index: target,
                        depth: *depth,
                    }
                }
                Source::Texture { location } => {
                    images.push(self.image(device, queue, location)?);
                    Src::Image(images.len() - 1)
                }
            };
            refs.push(InputRef {
                src,
                bilinear: input.bilinear,
            });
        }
        let mut pass = Pass::new(device, "", "", [(0, false); 2], output);
        let mut entries = Vec::new();
        let mut blocks = Vec::new();
        for (name, binding) in &program.blocks {
            let block = match name.as_str() {
                "SamplerInfo" => Block::Info,
                "Globals" => Block::Globals,
                other => {
                    let values = p
                        .uniforms
                        .get(other)
                        .ok_or(format!("pass {n}: the uniform block {other} is missing"))?;
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Post effect uniforms"),
                        size: UNIFORM_BYTES,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &pack_block(values)?);
                    Block::Custom(buffer)
                }
            };
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: *binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
            blocks.push((*binding, block));
        }
        let mut samplers = Vec::new();
        for (name, texture) in &program.samplers {
            let input = name.strip_suffix("Sampler").unwrap_or(name);
            let slot = p
                .inputs
                .iter()
                .position(|i| i.sampler == input)
                .ok_or(format!("pass {n}: no input for the sampler {name}"))?;
            // A depth buffer is read unfiltered.
            let depth = matches!(refs[slot].src, Src::Target { depth: true, .. });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: *texture,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: !depth },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: texture + 1,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(if depth {
                    wgpu::SamplerBindingType::NonFiltering
                } else {
                    wgpu::SamplerBindingType::Filtering
                }),
                count: None,
            });
            samplers.push((*texture, slot));
        }
        let module = |module: naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Post effect shader"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
            })
        };
        pass.refs = refs;
        pass.translated = Some(Translated {
            layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Post effect bindings"),
                entries: &entries,
            }),
            vertex: module(program.vertex),
            fragment: module(program.fragment),
            pipelines: HashMap::new(),
            blocks,
            samplers,
        });
        Ok(pass)
    }

    /// A pass over Horizon's WGSL stand-ins for the vanilla post shaders.
    fn standin_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        n: usize,
        p: &PassConfig,
        output: usize,
        index: &dyn Fn(&str) -> Option<usize>,
    ) -> Result<Pass, String> {
        let &(_, vs, vs_block) = VERTEX
            .iter()
            .find(|v| v.0 == p.vertex)
            .ok_or(format!("pass {n}: no vertex shader {}", p.vertex))?;
        let &(_, fs, fs_block, samplers) = FRAGMENT
            .iter()
            .find(|f| f.0 == p.fragment)
            .ok_or(format!("pass {n}: no fragment shader {}", p.fragment))?;
        let mut inputs = [(0, false); 2];
        for (slot, name) in SAMPLERS.iter().enumerate().take(samplers) {
            let input = p
                .inputs
                .iter()
                .find(|i| i.sampler == *name)
                .ok_or(format!("pass {n}: {} needs the sampler {name}", p.fragment))?;
            let Source::Target { id, depth: false } = &input.source else {
                return Err(format!(
                    "pass {n}: the built-in shaders read target colours"
                ));
            };
            let target = index(id).ok_or("a target is missing")?;
            if target == output {
                return Err(format!("pass {n}: reads the target it writes"));
            }
            inputs[slot] = (target, input.bilinear);
        }
        // A shader reading one sampler binds the same texture to the other,
        // never the target it writes.
        if samplers == 1 {
            inputs[1] = inputs[0];
        }
        let block = |name: &str| match p.uniforms.get(name) {
            Some(values) => pack_block(values),
            None if name.is_empty() => Ok([0; BLOCK_BYTES]),
            None => Err(format!("pass {n}: the uniform block {name} is missing")),
        };
        let pass = Pass::new(device, vs, fs, inputs, output);
        queue.write_buffer(&pass.vs_block, 0, &block(vs_block)?);
        queue.write_buffer(&pass.fs_block, 0, &block(fs_block)?);
        Ok(pass)
    }

    /// `textures/effect/<location>.png` as a texture (`PostChain.createPass`'s `TextureInput`).
    fn image(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        location: &str,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, [u32; 2]), String> {
        let (ns, path) = location.split_once(':').unwrap_or(("minecraft", location));
        let file = self
            .root
            .join("assets")
            .join(ns)
            .join("textures/effect")
            .join(format!("{path}.png"));
        let mut decoder = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(&file).map_err(|e| format!("{}: {e}", file.display()))?,
        ));
        // As the other resource-pack images: stb_image never checks chunk CRCs.
        decoder.ignore_checksums(true);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut data = vec![0; reader.output_buffer_size().ok_or("a huge image")?];
        let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => data[..info.buffer_size()].to_vec(),
            png::ColorType::Rgb => data[..info.buffer_size()]
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => data[..info.buffer_size()]
                .iter()
                .flat_map(|g| [*g, *g, *g, 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => data[..info.buffer_size()]
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => return Err("an indexed image".into()),
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Post effect image"),
            size: wgpu::Extent3d {
                width: info.width,
                height: info.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(info.width * 4),
                rows_per_image: Some(info.height),
            },
            wgpu::Extent3d {
                width: info.width,
                height: info.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        Ok((texture, view, [info.width, info.height]))
    }

    fn texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Post effect target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        (texture, view)
    }

    /// Runs the chains `req` names over `frame` (the frame's texture, copied from)
    /// and puts the result on `target` (its view): `GameRenderer.preparePostEffects`
    /// then `applyPostEffects`. `frame` must allow `COPY_SRC`. `depth` is the frame's
    /// depth buffer, for a pass that reads the depth of `minecraft:main` (it must
    /// allow `TEXTURE_BINDING`; without it that depth reads as 1). Returns whether
    /// any chain ran.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        target: &wgpu::TextureView,
        size: [u32; 2],
        depth: Option<&wgpu::TextureView>,
        req: &Request,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) -> bool {
        self.use_root(req.root);
        let mut applied = Vec::new();
        for effect in req.effects {
            let id = normalise(effect);
            if self.failed.contains(&id) || applied.contains(&id) {
                continue;
            }
            if !self.chains.contains_key(&id) {
                let loaded = match self.load(device, queue, &id) {
                    Ok(chain) => chain,
                    Err(e) => {
                        log::warn!("Failed to load post effect {id}: {e}");
                        self.failed.insert(id.clone());
                        None
                    }
                };
                self.chains.insert(id.clone(), loaded);
            }
            if matches!(self.chains.get(&id), Some(Some(_))) {
                applied.push(id);
            }
        }
        if applied.is_empty() {
            return false;
        }
        let format = frame.format().remove_srgb_suffix();
        if self.work.as_ref().is_none_or(|(t, _)| {
            t.size().width != size[0] || t.size().height != size[1] || t.format() != format
        }) {
            self.work = Some(Self::texture(device, format, size));
        }
        if self.staging.as_ref().is_none_or(|(t, _)| {
            t.size().width != size[0] || t.size().height != size[1] || t.format() != format
        }) {
            self.staging = Some(Self::texture(device, format, size));
        }
        queue.write_buffer(
            &self.globals,
            0,
            &req.globals.bytes(size, req.menu_blur_radius as i32),
        );
        if self.blit.is_none() {
            let pass = Pass::new(device, "vs_quad", "fs_blit", [(0, false); 2], 0);
            queue.write_buffer(&pass.vs_block, 0, &[0; BLOCK_BYTES]);
            queue.write_buffer(
                &pass.fs_block,
                0,
                &[1.0f32; 32].map(f32::to_le_bytes).concat(),
            );
            self.blit = Some(pass);
        }
        let copy_size = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let work = self.work.take().expect("made above");
        let staging = self.staging.take().expect("made above");
        let blit = self.blit.take().expect("made above");
        // The chains see GL's rows: flip the frame in, and back out below.
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            staging.0.as_image_copy(),
            copy_size,
        );
        self.run_pass(
            device,
            queue,
            encoder,
            &blit,
            [(&staging.1, size), (&staging.1, size)],
            (&work.1, size),
            0.0,
            format,
            timing.as_deref_mut(),
        );
        self.staging = Some(staging);
        for id in &applied {
            let mut chain = self.chains.remove(id).flatten().expect("loaded above");
            self.run_chain(
                device,
                queue,
                encoder,
                &work.1,
                &mut chain,
                size,
                depth,
                req.menu_blur_radius,
                format,
                timing.as_deref_mut(),
            );
            self.chains.insert(id.clone(), Some(chain));
        }
        // The frame, as the chains left it.
        self.run_pass(
            device,
            queue,
            encoder,
            &blit,
            [(&work.1, size), (&work.1, size)],
            (target, size),
            0.0,
            format,
            timing,
        );
        self.blit = Some(blit);
        self.work = Some(work);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn run_chain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        main: &wgpu::TextureView,
        chain: &mut Chain,
        size: [u32; 2],
        main_depth: Option<&wgpu::TextureView>,
        menu_radius: f32,
        format: wgpu::TextureFormat,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        for pass in &mut chain.passes {
            if let Some(translated) = &mut pass.translated {
                translated.ensure_pipeline(device, format);
            }
        }
        let sizes: Vec<[u32; 2]> = chain
            .targets
            .iter()
            .map(|(_, t)| [t.width.unwrap_or(size[0]), t.height.unwrap_or(size[1])])
            .collect();
        // PostChain.addToFrame: internal targets at their size, cleared to their
        // colour; a persistent one only when it is made.
        let remade = chain.size != size || chain.textures.len() + 1 != chain.targets.len();
        if remade {
            chain.size = size;
            chain.textures = sizes[1..]
                .iter()
                .map(|s| Self::texture(device, format, *s))
                .collect();
            let mut wanted = vec![false; chain.targets.len()];
            for r in chain.passes.iter().flat_map(|p| &p.refs) {
                if let Src::Target { index, depth: true } = r.src {
                    wanted[index] = true;
                }
            }
            chain.depths = wanted
                .iter()
                .zip(&sizes)
                .map(|(wanted, s)| {
                    wanted.then(|| {
                        let texture = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("Post effect depth"),
                            size: wgpu::Extent3d {
                                width: s[0],
                                height: s[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Depth32Float,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        });
                        let view = texture.create_view(&Default::default());
                        (texture, view)
                    })
                })
                .collect();
        }
        // The depth of a target nothing draws depth into stays at the far plane.
        for (i, depth) in chain.depths.iter().enumerate() {
            if let Some((_, view)) = depth
                && !(i == 0 && main_depth.is_some())
            {
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Post effect depth clear"),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("Post clear")),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }
        }
        for (i, (_, t)) in chain.targets.iter().enumerate().skip(1) {
            if remade || !t.persistent {
                let c = t.clear.map(f64::from);
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Post effect target clear"),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("Post clear")),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &chain.textures[i - 1].1,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                                a: c[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
        }
        let chain = &*chain;
        let view = |i: usize| {
            if i == 0 {
                main
            } else {
                &chain.textures[i - 1].1
            }
        };
        let depth_view = |i: usize| match (i, main_depth) {
            (0, Some(depth)) => depth,
            _ => &chain.depths[i].as_ref().expect("made for the pass").1,
        };
        for pass in &chain.passes {
            if let Some(translated) = &pass.translated {
                let inputs: Vec<(&wgpu::TextureView, [u32; 2])> = pass
                    .refs
                    .iter()
                    .map(|r| match r.src {
                        Src::Target {
                            index,
                            depth: false,
                        } => (view(index), sizes[index]),
                        Src::Target { index, depth: true } => (depth_view(index), sizes[index]),
                        Src::Image(i) => (&chain.images[i].1, chain.images[i].2),
                    })
                    .collect();
                self.draw_translated(
                    device,
                    queue,
                    encoder,
                    pass,
                    translated,
                    &inputs,
                    (view(pass.output), sizes[pass.output]),
                    format,
                    timing.as_deref_mut(),
                );
                continue;
            }
            let [a, b] = pass.inputs.map(|(t, _)| (view(t), sizes[t]));
            self.run_pass(
                device,
                queue,
                encoder,
                pass,
                [a, b],
                (view(pass.output), sizes[pass.output]),
                menu_radius,
                format,
                timing.as_deref_mut(),
            );
        }
    }

    /// `PostPass.addToFrame`'s execution for a translated pass.
    #[allow(clippy::too_many_arguments)]
    fn draw_translated(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        translated: &Translated,
        inputs: &[(&wgpu::TextureView, [u32; 2])],
        output: (&wgpu::TextureView, [u32; 2]),
        format: wgpu::TextureFormat,
        timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        let Some(pipeline) = translated.pipelines.get(&format) else {
            return;
        };
        // SamplerInfo: OutSize, then each input's size.
        let sizes = std::iter::once(output.1).chain(inputs.iter().map(|i| i.1));
        let info: Vec<u8> = sizes
            .flat_map(|s| s.map(|v| (v as f32).to_le_bytes()))
            .flatten()
            .collect();
        queue.write_buffer(
            &pass.info,
            0,
            &info[..info.len().min(UNIFORM_BYTES as usize)],
        );
        let mut entries = Vec::new();
        for (binding, block) in &translated.blocks {
            let buffer = match block {
                Block::Info => &pass.info,
                Block::Globals => &self.globals,
                Block::Custom(buffer) => buffer,
            };
            entries.push(wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            });
        }
        for (binding, slot) in &translated.samplers {
            let ref_ = &pass.refs[*slot];
            let depth = matches!(ref_.src, Src::Target { depth: true, .. });
            entries.push(wgpu::BindGroupEntry {
                binding: *binding,
                resource: wgpu::BindingResource::TextureView(inputs[*slot].0),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(if ref_.bilinear && !depth {
                    &self.bilinear
                } else {
                    &self.nearest
                }),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Post effect pass"),
            layout: &translated.layout,
            entries: &entries,
        });
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post effect pass"),
            timestamp_writes: timing.and_then(|t| t.writes("Post effect")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
    }

    /// `PostPass.addToFrame`'s execution: one fullscreen triangle.
    #[allow(clippy::too_many_arguments)]
    fn run_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        inputs: [(&wgpu::TextureView, [u32; 2]); 2],
        output: (&wgpu::TextureView, [u32; 2]),
        menu_radius: f32,
        format: wgpu::TextureFormat,
        timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        // SamplerInfo: OutSize, then each input's size.
        let info = [
            output.1[0] as f32,
            output.1[1] as f32,
            inputs[0].1[0] as f32,
            inputs[0].1[1] as f32,
            inputs[1].1[0] as f32,
            inputs[1].1[1] as f32,
            menu_radius,
            0.0,
        ];
        queue.write_buffer(&pass.info, 0, &info.map(f32::to_le_bytes).concat());
        self.pipeline(device, (pass.vs, pass.fs), format);
        let Some(pipeline) = self.pipelines.get(&(pass.vs, pass.fs, format)) else {
            return;
        };
        let sampler = |bilinear| {
            if bilinear {
                &self.bilinear
            } else {
                &self.nearest
            }
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Post effect pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pass.info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pass.vs_block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: pass.fs_block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs[0].0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(sampler(pass.inputs[0].1)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(inputs[1].0),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(sampler(pass.inputs[1].1)),
                },
            ],
        });
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post effect pass"),
            timestamp_writes: timing.and_then(|t| t.writes("Post effect")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
    }
}

impl Pass {
    fn new(
        device: &wgpu::Device,
        vs: &'static str,
        fs: &'static str,
        inputs: [(usize, bool); 2],
        output: usize,
    ) -> Self {
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Pass {
            vs,
            fs,
            inputs,
            output,
            info: buffer("Post effect sampler info", UNIFORM_BYTES),
            vs_block: buffer("Post effect vertex uniforms", BLOCK_BYTES as u64),
            fs_block: buffer("Post effect fragment uniforms", BLOCK_BYTES as u64),
            refs: Vec::new(),
            translated: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVERT_THEN_BLIT: &str = r#"{
        "targets": { "swap": {} },
        "passes": [
            { "vertex_shader": "minecraft:core/screenquad",
              "fragment_shader": "minecraft:post/invert",
              "inputs": [ { "sampler_name": "In", "target": "minecraft:main" } ],
              "output": "swap",
              "uniforms": { "InvertConfig": [ { "name": "InverseAmount", "type": "float", "value": 1.0 } ] } },
            { "vertex_shader": "minecraft:core/screenquad",
              "fragment_shader": "minecraft:post/blit",
              "inputs": [ { "sampler_name": "In", "target": "swap", "bilinear": true } ],
              "output": "minecraft:main",
              "uniforms": { "BlitConfig": [ { "name": "ColorModulate", "type": "vec4", "value": [1.0, 0.0, 0.5, 1.0] } ] } }
        ]
    }"#;

    #[test]
    fn shaders_validate() {
        let module = naga::front::wgsl::parse_str(include_str!("post_chain.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn a_chain_is_read_like_postchainconfig() {
        let config = Config::parse(INVERT_THEN_BLIT).unwrap();
        // Ids without a namespace are minecraft's.
        assert_eq!(
            config.targets.keys().collect::<Vec<_>>(),
            ["minecraft:swap"]
        );
        assert_eq!(config.passes.len(), 2);
        assert_eq!(config.passes[0].output, "minecraft:swap");
        assert_eq!(
            config.passes[1].inputs,
            [Input {
                sampler: "In".into(),
                source: Source::Target {
                    id: "minecraft:swap".into(),
                    depth: false
                },
                bilinear: true
            }]
        );
        assert_eq!(
            config.passes[1].uniforms["BlitConfig"],
            [Uniform::Vec4([1.0, 0.0, 0.5, 1.0])]
        );
        // Only the frame is outside the chain.
        assert_eq!(config.external_targets(), HashSet::from(["minecraft:main"]));
    }

    #[test]
    fn target_options_and_clear_colour() {
        let config = Config::parse(
            r#"{"targets":{"a":{"width":16,"height":8,"persistent":true,"clear_color":2180972544}}}"#,
        )
        .unwrap();
        let t = &config.targets["minecraft:a"];
        assert_eq!((t.width, t.height, t.persistent), (Some(16), Some(8), true));
        // 0x81FF0000: alpha 129, red 255.
        assert_eq!(t.clear, [1.0, 0.0, 0.0, 129.0 / 255.0]);
        assert!(Config::parse(r#"{"targets":{"a":{"width":0}}}"#).is_err());
    }

    #[test]
    fn inputs_are_checked() {
        let pass = |inputs: &str| {
            format!(
                r#"{{"passes":[{{"vertex_shader":"a","fragment_shader":"b","output":"o","inputs":{inputs}}}]}}"#
            )
        };
        // Repeated sampler names are refused.
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","target":"x"},{"sampler_name":"In","target":"y"}]"#
            ))
            .is_err()
        );
        // A texture input needs its size; an input is a target or a texture, not both.
        assert!(Config::parse(&pass(r#"[{"sampler_name":"In","location":"noise"}]"#)).is_err());
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","location":"noise","target":"x","width":4,"height":4}]"#
            ))
            .is_err()
        );
        let config = Config::parse(&pass(
            r#"[{"sampler_name":"A","location":"noise","width":4,"height":4,"bilinear":true},
                {"sampler_name":"B","target":"x","use_depth_buffer":true}]"#,
        ))
        .unwrap();
        assert_eq!(
            config.passes[0].inputs[0].source,
            Source::Texture {
                location: "minecraft:noise".into()
            }
        );
        assert_eq!(
            config.passes[0].inputs[1].source,
            Source::Target {
                id: "minecraft:x".into(),
                depth: true
            }
        );
        // Only targets can be external.
        assert_eq!(
            config.external_targets(),
            HashSet::from(["minecraft:x", "minecraft:o"])
        );
    }

    #[test]
    fn blocks_are_packed_as_std140() {
        // BlurConfig { vec2 BlurDir; float Radius }.
        let block = pack_block(&[Uniform::Vec2([1.0, 0.0]), Uniform::Float(7.0)]).unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..3], [1.0, 0.0, 7.0]);
        // ColorConfig { vec3, vec3, vec3 }: each starts on 16 bytes.
        let block = pack_block(&[
            Uniform::Vec3([1.0, 2.0, 3.0]),
            Uniform::Vec3([4.0, 5.0, 6.0]),
        ])
        .unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..7], [1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0]);
        // RotScaleConfig { vec2, vec2, float }.
        let block = pack_block(&[
            Uniform::Vec2([1.0, 2.0]),
            Uniform::Vec2([3.0, 4.0]),
            Uniform::Float(5.0),
        ])
        .unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..5], [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(pack_block(&[Uniform::Matrix([0.0; 16]); 3]).is_err());
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn scratch(name: &str, json: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-post-chain-{}-{name}", std::process::id()));
        let dir = root.join("assets/minecraft/post_effect");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.json")), json).unwrap();
        root
    }

    fn run(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effects: &mut PostEffects,
        root: &Path,
        names: &[String],
    ) -> (bool, [u8; 4]) {
        let frame = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = frame.create_view(&Default::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        // The frame drawn so far: (0.2, 0.4, 0.6).
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.2,
                        g: 0.4,
                        b: 0.6,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let ran = effects.apply(
            device,
            queue,
            &mut encoder,
            &frame,
            &view,
            [8, 4],
            None,
            &Request {
                effects: names,
                root,
                menu_blur_radius: 5.0,
                globals: Globals::default(),
            },
            None,
        );
        let buffer = crate::gfx::copy_frame(device, &mut encoder, &frame, [8, 4]);
        queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("{error}");
        }
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        (ran, [data[0], data[1], data[2], data[3]])
    }

    /// The chain runs over the frame and the result lands on it: invert into
    /// `swap`, then a blit modulated by (1, 0, 0.5, 1) back onto the frame.
    #[test]
    fn a_chain_draws_onto_the_frame() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch("invert_then_blit", INVERT_THEN_BLIT);
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:invert_then_blit".to_owned()];
        let (ran, px) = run(&device, &queue, &mut effects, &root, &names);
        assert!(ran);
        // (0.8, 0.6, 0.4) * (1, 0, 0.5): 204, 0, 51.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 2;
        assert!(
            near(px[0], 204) && px[1] == 0 && near(px[2], 51) && px[3] == 255,
            "{px:?}"
        );
        // A second frame is the same (the chain and its targets are reused).
        let (_, again) = run(&device, &queue, &mut effects, &root, &names);
        assert_eq!(again, px);
        // Missing chains and a chain that does not load change nothing.
        let (ran, px) = run(
            &device,
            &queue,
            &mut effects,
            &root,
            &["minecraft:nothing".to_owned()],
        );
        assert!(!ran);
        assert!(
            near(px[0], 51) && near(px[1], 102) && near(px[2], 153),
            "{px:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_chain_naming_an_unknown_shader_fails_once() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch(
            "unknown",
            r#"{"passes":[{"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/nope",
                "inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"minecraft:main"}]}"#,
        );
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:unknown".to_owned()];
        assert!(!run(&device, &queue, &mut effects, &root, &names).0);
        assert!(effects.failed.contains("minecraft:unknown"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The chains the prepared vanilla resources hold all load (skipped without
    /// them): none names a shader Horizon lacks.
    /// Renders `names` over a frame whose top half is red and bottom half green,
    /// returning the pixel rows 0 and 3 of the 8x4 result.
    fn run_halves(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effects: &mut PostEffects,
        root: &Path,
        names: &[String],
    ) -> [[u8; 4]; 2] {
        let frame = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut pixels = Vec::new();
        for y in 0..4 {
            for _ in 0..8 {
                pixels.extend_from_slice(if y < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 255, 0, 255]
                });
            }
        }
        queue.write_texture(
            frame.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(4),
            },
            frame.size(),
        );
        let view = frame.create_view(&Default::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(effects.apply(
            device,
            queue,
            &mut encoder,
            &frame,
            &view,
            [8, 4],
            None,
            &Request {
                effects: names,
                root,
                menu_blur_radius: 0.0,
                globals: Globals::default(),
            },
            None,
        ));
        let buffer = crate::gfx::copy_frame(device, &mut encoder, &frame, [8, 4]);
        queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("{error}");
        }
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        let row = |y: usize| {
            let at = y * 256;
            [data[at], data[at + 1], data[at + 2], data[at + 3]]
        };
        [row(0), row(3)]
    }

    /// Frame orientation survives a chain through an intermediate target.
    #[test]
    fn a_chain_keeps_the_frame_upright() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch(
            "upright",
            r#"{"targets":{"swap":{}},"passes":[
            {"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/blit",
             "inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"swap",
             "uniforms":{"BlitConfig":[{"name":"ColorModulate","type":"vec4","value":[1.0,1.0,1.0,1.0]}]}},
            {"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/blit",
             "inputs":[{"sampler_name":"In","target":"swap"}],"output":"minecraft:main",
             "uniforms":{"BlitConfig":[{"name":"ColorModulate","type":"vec4","value":[1.0,1.0,1.0,1.0]}]}}]}"#,
        );
        let mut effects = PostEffects::new(&device);
        let [top, bottom] = run_halves(
            &device,
            &queue,
            &mut effects,
            &root,
            &["minecraft:upright".to_owned()],
        );
        assert_eq!(top, [255, 0, 0, 255]);
        assert_eq!(bottom, [0, 255, 0, 255]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A chain with its own GLSL, a target read twice, a depth input and a
    /// `textures/effect` image: nothing here is one of vanilla's shaders.
    #[test]
    fn a_chain_runs_its_own_glsl() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch(
            "custom",
            r#"{"targets":{"half":{"width":4,"height":2}},"passes":[
            {"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/mine",
             "inputs":[{"sampler_name":"In","target":"minecraft:main","bilinear":true},
                       {"sampler_name":"Depth","target":"minecraft:main","use_depth_buffer":true},
                       {"sampler_name":"Tint","location":"tint","width":1,"height":1}],
             "output":"half",
             "uniforms":{"MineConfig":[{"name":"Scale","type":"float","value":0.5}]}},
            {"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/mine",
             "inputs":[{"sampler_name":"In","target":"half"},
                       {"sampler_name":"Depth","target":"half","use_depth_buffer":true},
                       {"sampler_name":"Tint","location":"tint","width":1,"height":1}],
             "output":"minecraft:main",
             "uniforms":{"MineConfig":[{"name":"Scale","type":"float","value":1.0}]}}]}"#,
        );
        let shaders = root.join("assets/minecraft/shaders");
        std::fs::create_dir_all(shaders.join("core")).unwrap();
        std::fs::create_dir_all(shaders.join("post")).unwrap();
        std::fs::write(
            shaders.join("core/quad.vsh"),
            "#version 330\nlayout(location = 0) out vec2 texCoord;\nvoid main() {\n  vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n  gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n  texCoord = uv;\n}\n",
        )
        .unwrap();
        std::fs::write(
            shaders.join("post/mine.fsh"),
            "#version 330\nuniform sampler2D InSampler;\nuniform sampler2D DepthSampler;\nuniform sampler2D TintSampler;\n\
             layout(std140) uniform SamplerInfo { vec2 OutSize; vec2 InSize; };\n\
             layout(std140) uniform MineConfig { float Scale; };\n\
             layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n  vec4 c = texture(InSampler, texCoord);\n  float d = texture(DepthSampler, texCoord).r;\n\
               vec4 t = texture(TintSampler, texCoord);\n  fragColor = vec4(c.rgb * t.rgb * Scale * d, 1.0);\n}\n",
        )
        .unwrap();
        // A 1x1 image, (0.5, 1, 1).
        let effect = root.join("assets/minecraft/textures/effect");
        std::fs::create_dir_all(&effect).unwrap();
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[128, 255, 255, 255]).unwrap();
        }
        std::fs::write(effect.join("tint.png"), png_bytes).unwrap();
        let mut effects = PostEffects::new(&device);
        let [top, bottom] = run_halves(
            &device,
            &queue,
            &mut effects,
            &root,
            &["minecraft:custom".to_owned()],
        );
        // Red (1, 0, 0) * the image (0.5, 1, 1) * 0.5 * depth 1, then again * 1: 0.125.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 3;
        assert!(near(top[0], 32) && top[1] == 0 && top[3] == 255, "{top:?}");
        // Green stays below it: 0.5 * 1 * 1.
        assert!(bottom[0] == 0 && near(bottom[1], 128), "{bottom:?}");
        assert!(effects.failed.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Vanilla's own chains run on the GPU without a validation error (skipped
    /// without the prepared resources).
    #[test]
    fn vanilla_chains_run() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !root
            .join("assets/minecraft/post_effect/invert.json")
            .exists()
        {
            return;
        }
        for name in ["blur", "creeper", "invert", "spider"] {
            let mut effects = PostEffects::new(&device);
            run_halves(
                &device,
                &queue,
                &mut effects,
                &root,
                &[format!("minecraft:{name}")],
            );
            assert!(effects.failed.is_empty(), "{name}");
            // The chain ran on the resources' own GLSL, not the stand-ins.
            let chain = effects.chains[&format!("minecraft:{name}")]
                .as_ref()
                .unwrap();
            assert!(
                chain.passes.iter().all(|p| p.translated.is_some()),
                "{name}"
            );
        }
    }

    #[test]
    fn vanilla_chains_load() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !root
            .join("assets/minecraft/post_effect/invert.json")
            .exists()
        {
            return;
        }
        let mut effects = PostEffects::new(&device);
        effects.use_root(&root);
        for name in ["blur", "creeper", "invert", "spider"] {
            let id = format!("minecraft:{name}");
            assert!(
                effects.load(&device, &queue, &id).unwrap().is_some(),
                "{id}"
            );
        }
        // entity_outline needs targets and shaders the world pass has of its own.
        assert!(
            effects
                .load(&device, &queue, "minecraft:entity_outline")
                .is_err()
        );
    }
}
