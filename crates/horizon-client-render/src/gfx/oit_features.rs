//! Improved transparency for what is drawn after the opaque world besides the
//! terrain and the clouds: `LevelRenderer.executeOit`'s "Features, World
//! Border, Weather" pass. Each kind keeps its classic vertex shader, vertex
//! format and depth state (minus the depth writes, as every `OIT_*` pipeline),
//! with the three stages' fragments (`oit_features.wgsl`, `oit_border.wgsl`).

use super::oit::{Stage, stage_targets};

/// One pipeline per stage.
type Set = [wgpu::RenderPipeline; 3];

pub struct Features {
    /// `OIT_ENTITY`/`OIT_ITEM`: translucent items and particles.
    pub entity: Set,
    /// `OIT_TEXT_POLYGON_OFFSET`.
    pub text: Set,
    /// `OIT_CRUMBLING`.
    pub crumbling: Set,
    /// `OIT_ENTITY_SHADOW`.
    pub shadow: Set,
    /// `OIT_ENERGY_SWIRL`.
    pub swirl: Set,
    /// `OIT_LIGHTNING`: the retained bolts.
    pub lightning: Set,
    /// `OIT_DRAGON_RAYS`: the additive geometry after the bolts.
    pub additive: Set,
    /// `OIT_WEATHER`.
    pub weather: Set,
    /// `OIT_WORLD_BORDER`.
    pub border: Set,
}

/// What a kind differs in.
struct Kind<'a> {
    label: &'a str,
    /// The entry points' common prefix: `fs_oit_<name>_<stage>`.
    name: &'a str,
    vertex: &'a str,
    cull: Option<wgpu::Face>,
    bias: wgpu::DepthBiasState,
}

const PLAIN: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 0,
    slope_scale: 0.0,
    clamp: 0.0,
};

impl Features {
    /// `layouts`: `Oit`'s stage groups (at group 2); `entity`, `weather` and
    /// `border` the bind group layouts of the classic pipelines.
    pub fn new(
        device: &wgpu::Device,
        layouts: &[wgpu::BindGroupLayout; 4],
        entity: &wgpu::BindGroupLayout,
        weather: &wgpu::BindGroupLayout,
        border: &wgpu::BindGroupLayout,
    ) -> Self {
        let entity_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon improved transparency features"),
            source: wgpu::ShaderSource::Wgsl(
                (super::entity_shader()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_features.wgsl"))
                .into(),
            ),
        });
        let border_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon improved transparency world border"),
            source: wgpu::ShaderSource::Wgsl(
                (include_str!("../border.wgsl").to_owned()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_border.wgsl"))
                .into(),
            ),
        });
        let entity_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let weather_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Float32x4,
            3=>Float32x4,4=>Uint32x4,5=>Unorm8x4,6=>Uint8x4,7=>Uint32];
        let border_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2];
        let entity_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &entity_attributes,
        })];
        let weather_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::weather::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &weather_attributes,
        })];
        let border_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::border_render::BorderVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &border_attributes,
        })];
        let lightning_attributes = wgpu::vertex_attr_array![7=>Float32x3];
        let lightning_buffers = [
            entity_buffers[0].clone(),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &lightning_attributes,
            }),
        ];
        let set = |module: &wgpu::ShaderModule,
                   groups: [Option<&wgpu::BindGroupLayout>; 2],
                   buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                   kind: Kind<'_>| {
            [Stage::DepthBounds, Stage::Transmittance, Stage::Accumulate].map(|stage| {
                let suffix = ["bounds", "transmittance", "accumulate"][stage as usize];
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Improved transparency features"),
                    bind_group_layouts: &[groups[0], groups[1], Some(&layouts[stage as usize])],
                    ..Default::default()
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&format!("OIT {} {suffix}", kind.label)),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module,
                        entry_point: Some(kind.vertex),
                        compilation_options: Default::default(),
                        buffers,
                    },
                    primitive: wgpu::PrimitiveState {
                        cull_mode: kind.cull,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                        stencil: Default::default(),
                        bias: kind.bias,
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module,
                        entry_point: Some(&format!("fs_oit_{}_{suffix}", kind.name)),
                        compilation_options: Default::default(),
                        targets: &stage_targets(stage),
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
        };
        let entities = [Some(entity), None];
        let kind = |label, name, cull, bias| Kind {
            label,
            name,
            vertex: "vs_entity",
            cull,
            bias,
        };
        // The classic pipelines' biases: text and cracks (10, 1), shadows
        // (128 ulps and a slope, over the block tops they lie on).
        let offset = wgpu::DepthBiasState {
            constant: 10,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let shadow_bias = wgpu::DepthBiasState {
            constant: 128,
            slope_scale: 0.015,
            clamp: 0.0,
        };
        Self {
            entity: set(
                &entity_module,
                entities,
                &entity_buffers,
                kind("entities", "entity", None, PLAIN),
            ),
            text: set(
                &entity_module,
                entities,
                &entity_buffers,
                kind("text", "entity", Some(wgpu::Face::Back), offset),
            ),
            crumbling: set(
                &entity_module,
                entities,
                &entity_buffers,
                kind("breaking cracks", "entity", Some(wgpu::Face::Back), offset),
            ),
            shadow: set(
                &entity_module,
                entities,
                &entity_buffers,
                kind(
                    "entity shadows",
                    "shadow",
                    Some(wgpu::Face::Back),
                    shadow_bias,
                ),
            ),
            swirl: set(
                &entity_module,
                entities,
                &entity_buffers,
                kind("energy swirls", "swirl", None, PLAIN),
            ),
            lightning: set(
                &entity_module,
                entities,
                &lightning_buffers,
                Kind {
                    label: "lightning",
                    name: "lightning",
                    vertex: "vs_lightning",
                    cull: None,
                    bias: PLAIN,
                },
            ),
            additive: set(
                &entity_module,
                entities,
                &entity_buffers,
                kind(
                    "additive entities",
                    "lightning",
                    Some(wgpu::Face::Back),
                    PLAIN,
                ),
            ),
            weather: set(
                &entity_module,
                [Some(entity), Some(weather)],
                &weather_buffers,
                Kind {
                    label: "weather",
                    name: "weather",
                    vertex: "vs_weather",
                    cull: None,
                    bias: PLAIN,
                },
            ),
            // `OIT_WORLD_BORDER`: a polygon offset of 3 towards the camera.
            border: set(
                &border_module,
                [Some(border), None],
                &border_buffers,
                Kind {
                    label: "world border",
                    name: "border",
                    vertex: "vs_border",
                    cull: None,
                    bias: wgpu::DepthBiasState {
                        constant: 3,
                        slope_scale: 3.0,
                        clamp: 0.0,
                    },
                },
            ),
        }
    }
}
