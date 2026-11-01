//! Entities, block entities, translucent entity parts and particles drawn by a shader pack's
//! `gbuffers_entities`, `gbuffers_block`, `gbuffers_entities_translucent` and
//! `gbuffers_particles` (spec 2.3, 5.3, 5.4): the runs of `Frame::entities` the game marked (`EntitySpan`), each its
//! own draw with its `entityId` or `blockEntityId` and `entityColor` in a per-draw slot of
//! `HzDraw` (set 2). Horizon draws whatever the pack does not.

use std::ops::Range;

use horizon_shaders::render_stage;

use super::Vertex;
use super::pack_draw::EntitySpan;

/// `HzDraw`'s size (pack_draw.rs).
const SLOT: u64 = 32;

const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x2, 2 => Uint32, 3 => Unorm8x4, 4 => Uint8x4, 5 => Snorm8x4
];

/// The entity vertex: Horizon's own (`Vertex`).
pub fn buffers() -> [Option<wgpu::VertexBufferLayout<'static>>; 1] {
    [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    })]
}

/// What `gbuffers_entities` and `gbuffers_block` read from an entity vertex: camera-relative
/// position, the shared atlas's UV, colour, light (block, sky in 16ths) and the world-oriented
/// normal. The quad's corners, read from the same vertices at set 1, give the face normal where
/// the vertex has none (block entities) and `at_tangent` along the texture's u, as for terrain.
pub fn inputs() -> horizon_shaders::GbuffersInputs {
    let declarations = "layout(location = 0) in vec3 hz_v_pos;\nlayout(location = 1) in vec2 hz_v_uv;\n\
         layout(location = 2) in uint hz_v_overlay;\nlayout(location = 3) in vec4 hz_v_color;\n\
         layout(location = 4) in uvec4 hz_v_light;\nlayout(location = 5) in vec4 hz_v_normal;\n\
         layout(set = 1, binding = 0) readonly buffer HzEntityVertices { uint hz_entity_vertices[]; };\n\
         vec4 hz_tangent; vec2 hz_mid;\n"
        .to_owned();
    let words = std::mem::size_of::<Vertex>() / 4;
    let corner = |k: usize, field: &str| {
        let at = |o: usize| format!("hz_entity_vertices[hz_q + {}u]", k * words + o);
        match field {
            "pos" => format!("uintBitsToFloat(uvec3({}, {}, {}))", at(0), at(1), at(2)),
            _ => format!("uintBitsToFloat(uvec2({}, {}))", at(3), at(4)),
        }
    };
    let body = format!(
        "    uint hz_q = (uint(gl_VertexIndex) & ~3u) * {words}u;\n    \
         vec3 hz_e1 = {p1} - {p0};\n    \
         vec3 hz_e2 = {p2} - {p0};\n    \
         vec2 hz_t0 = {t0};\n    \
         vec2 hz_d1 = {t1} - hz_t0;\n    \
         vec2 hz_d2 = {t2} - hz_t0;\n    \
         hz_mid = (hz_t0 + {t2}) * 0.5;\n    \
         vec3 hz_n = cross(hz_e1, hz_e2);\n    \
         hz_Normal = dot(hz_v_normal.xyz, hz_v_normal.xyz) > 0.01 ? normalize(hz_v_normal.xyz)\n        \
             : dot(hz_n, hz_n) > 1e-12 ? normalize(hz_n) : vec3(0.0, 1.0, 0.0);\n    \
         hz_tangent = vec4(1.0, 0.0, 0.0, 1.0);\n    \
         float hz_r = hz_d1.x * hz_d2.y - hz_d2.x * hz_d1.y;\n    \
         vec3 hz_t = (hz_e1 * hz_d2.y - hz_e2 * hz_d1.y) * sign(hz_r);\n    \
         vec3 hz_b = (hz_e2 * hz_d1.x - hz_e1 * hz_d2.x) * sign(hz_r);\n    \
         hz_t -= hz_Normal * dot(hz_Normal, hz_t);\n    \
         if (dot(hz_t, hz_t) > 1e-12) {{\n        \
             hz_t = normalize(hz_t);\n        \
             hz_tangent = vec4(hz_t, dot(cross(hz_t, hz_Normal), hz_b) < 0.0 ? -1.0 : 1.0);\n    \
         }}\n    \
         hz_Vertex = vec4(hz_v_pos, 1.0);\n    \
         hz_MultiTexCoord0 = vec4(hz_v_uv, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(float(hz_v_light.y), float(hz_v_light.x), 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n    \
         hz_Color = hz_v_color;\n",
        p0 = corner(0, "pos"),
        p1 = corner(1, "pos"),
        p2 = corner(2, "pos"),
        t0 = corner(0, "uv"),
        t1 = corner(1, "uv"),
        t2 = corner(2, "uv"),
    );
    let attributes = [
        ("at_tangent", "hz_tangent"),
        ("mc_midTexCoord", "vec4(hz_mid, 0.0, 1.0)"),
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaUV0", "hz_v_uv"),
        ("vaUV1", "ivec2(0, 10)"),
        ("vaUV2", "ivec2(hz_MultiTexCoord1.xy)"),
        ("vaColor", "hz_Color"),
        ("vaNormal", "hz_Normal"),
    ];
    horizon_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: attributes
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect(),
        after: String::new(),
        // Cutout, as vanilla's entity types mostly are (OQ-23): the loader's test at 0.1.
        alpha_test: Some(horizon_shaders::AlphaTest {
            op: ">",
            reference: 0.1,
        }),
    }
}

/// What `gbuffers_clouds` reads from the clouds' mesh (entity vertices, `vs_cloud`): the
/// position moved by the clouds' offset, the colour of the face's shade from their palette,
/// full light, the face's normal. Set 1 holds the clouds' uniform (`clouds::Uniform`).
pub fn cloud_inputs() -> horizon_shaders::GbuffersInputs {
    let declarations = "layout(location = 0) in vec3 hz_v_pos;\nlayout(location = 1) in vec2 hz_v_uv;\n\
         layout(location = 2) in uint hz_v_overlay;\nlayout(location = 3) in vec4 hz_v_color;\n\
         layout(location = 4) in uvec4 hz_v_light;\nlayout(location = 5) in vec4 hz_v_normal;\n\
         layout(set = 1, binding = 0) uniform HzCloud { vec4 hz_cloud_offset; vec4 hz_cloud_colors[4]; };\n"
        .to_owned();
    let body = "    hz_Vertex = vec4(hz_v_pos + hz_cloud_offset.xyz, 1.0);\n    \
         hz_MultiTexCoord0 = vec4(hz_v_uv, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n    \
         hz_Color = hz_cloud_colors[min(hz_v_light.x, 3u)];\n    \
         hz_Normal = dot(hz_v_normal.xyz, hz_v_normal.xyz) > 0.01 ? normalize(hz_v_normal.xyz) : vec3(0.0, 1.0, 0.0);\n"
        .to_owned();
    let attributes = [
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaUV0", "hz_v_uv"),
        ("vaUV2", "ivec2(240, 240)"),
        ("vaColor", "hz_Color"),
        ("vaNormal", "hz_Normal"),
    ];
    horizon_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: attributes
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect(),
        after: String::new(),
        alpha_test: None,
    }
}

/// `entityColor` for a run whose first quad carries overlay code `overlay`: the hurt red or a
/// white flash, its alpha how much of that colour shows over the entity (vanilla mixes the
/// overlay texel's colour in by one minus the texel's alpha).
pub fn entity_color(overlay: u32) -> [f32; 4] {
    let [r, g, b, a] = crate::overlay::HURT_TEXEL.map(|v| f32::from(v) / 255.0);
    let white = |u: u32| {
        let a = f32::from(crate::overlay::white_alpha(u)) / 255.0;
        [1.0, 1.0, 1.0, 1.0 - a]
    };
    match overlay {
        1 | 8 => [r, g, b, 1.0 - a],
        5 => white(15),
        16..32 => white(overlay - crate::overlay::WHITE),
        _ => [0.0; 4],
    }
}

/// One draw: its quads in `PackEntities`' buffer, its program and its slot of `HzDraw`.
pub(super) struct Draw {
    /// The first vertex.
    pub base: u32,
    pub quads: u32,
    /// The kind of program that draws it (`ENTITY`, `BLOCK`, `TRANSLUCENT`, `PARTICLE`).
    pub program: usize,
    /// The pack's gbuffers program draws it (else Horizon does).
    pub pack: bool,
    /// The pack's shadow program draws it into the shadow map too.
    pub shadow: bool,
    pub slot: u32,
}

/// Which entity runs a pack's shadow pass draws (2.4, 3.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Casters {
    /// `shadow_entities` (or `shadow`) draws entities...
    pub entities: bool,
    /// ...only the local player (`shadowPlayer` without `shadowEntities`).
    pub only_player: bool,
    /// `shadow_block` (or `shadow`) draws block entities...
    pub blocks: bool,
    /// ...only those whose block emits light (`shadowLightBlockEntities`).
    pub only_lit: bool,
}

impl Casters {
    /// Whether the shadow pass draws `span`'s run, of kind `program`.
    pub fn cast(&self, span: &EntitySpan, program: usize) -> bool {
        match program {
            ENTITY => self.entities && (!self.only_player || span.player),
            BLOCK => self.blocks && (!self.only_lit || span.emissive),
            _ => false,
        }
    }
}

/// The vertices and per-draw uniforms of the runs a pack draws, rebuilt each frame.
pub(super) struct PackEntities {
    vertices: wgpu::Buffer,
    /// Set 1: the vertices again, for the quads' corners.
    pub table: wgpu::BindGroupLayout,
    pub table_group: wgpu::BindGroup,
    slots: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    stride: u64,
    /// Two triangles per quad, for `quads` quads.
    pub indices: wgpu::Buffer,
    quads: u32,
    scratch: Vec<Vertex>,
    uniforms: Vec<u8>,
    pub draws: Vec<Draw>,
}

impl PackEntities {
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let stride = SLOT.max(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));
        let table = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack entity vertices"),
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
        });
        let (vertices, table_group) = Self::vertex_buffer(device, &table, 4096);
        let (slots, group) = Self::slot_buffer(device, layout, 64, stride);
        Self {
            vertices,
            table,
            table_group,
            slots,
            group,
            stride,
            indices: quad_indices(device, 1024),
            quads: 1024,
            scratch: Vec::new(),
            uniforms: Vec::new(),
            draws: Vec::new(),
        }
    }

    fn vertex_buffer(
        device: &wgpu::Device,
        table: &wgpu::BindGroupLayout,
        vertices: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack entity vertices"),
            size: vertices * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack entity vertices"),
            layout: table,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        (buffer, group)
    }

    fn slot_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        slots: u64,
        stride: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack entity draws"),
            size: slots * stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack entity draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(SLOT),
                }),
            }],
        });
        (buffer, group)
    }

    pub fn vertices(&self) -> wgpu::BufferSlice<'_> {
        self.vertices.slice(..)
    }

    /// The dynamic offset of draw slot `slot`.
    pub fn offset(&self, slot: u32) -> u32 {
        (u64::from(slot) * self.stride) as u32
    }

    /// Splits `entities` between the pack, which draws the runs of `spans` its programs cover
    /// (`drawn`: whether it has `gbuffers_entities`, `gbuffers_block` or their fallbacks), and
    /// Horizon, which draws the rest (`rest`). Uploads the pack's part, and the runs its shadow
    /// programs draw (`shadow`). Runs hidden from the camera only ever cast shadows.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        entities: &[Vertex],
        spans: &[EntitySpan],
        drawn: [bool; PROGRAMS],
        shadow: Casters,
        rest: &mut Vec<Vertex>,
    ) {
        self.draws.clear();
        self.scratch.clear();
        self.uniforms.clear();
        rest.clear();
        let first = spans.first().map_or(entities.len(), |s| s.start as usize);
        rest.extend_from_slice(&entities[..first.min(entities.len())]);
        for (span, range) in runs(entities.len(), spans) {
            let run = &entities[range];
            let program = program_of(&span).filter(|_| plain(run));
            let pack = program.is_some_and(|p| drawn[p]) && !span.hidden;
            let casts = program.is_some_and(|p| shadow.cast(&span, p));
            if !pack && !span.hidden {
                rest.extend_from_slice(run);
            }
            let Some(program) = program.filter(|_| (pack || casts) && !run.is_empty()) else {
                continue;
            };
            let start = self.scratch.len() as u32;
            self.scratch.extend_from_slice(run);
            let (entity_id, block_id) = match program {
                BLOCK => (0, span.id),
                _ => (span.id, 0),
            };
            let mut bytes = [0u8; SLOT as usize];
            bytes[0..4].copy_from_slice(&(span.stage as i32).to_le_bytes());
            bytes[4..8].copy_from_slice(&entity_id.to_le_bytes());
            bytes[8..12].copy_from_slice(&block_id.to_le_bytes());
            // The overlay: the layer's low 16 bits (HDR output's tier rides in the high ones).
            for (i, c) in entity_color(run[0].layer & 0xffff).iter().enumerate() {
                bytes[16 + 4 * i..20 + 4 * i].copy_from_slice(&c.to_le_bytes());
            }
            self.uniforms.extend_from_slice(&bytes);
            self.uniforms
                .resize(self.uniforms.len() + (self.stride - SLOT) as usize, 0);
            self.draws.push(Draw {
                base: start,
                quads: run.len() as u32 / 4,
                program,
                pack,
                shadow: casts,
                slot: self.draws.len() as u32,
            });
        }
        if self.draws.is_empty() {
            return;
        }
        let vertex_size = std::mem::size_of::<Vertex>() as u64;
        let (need_vertices, need_slots) = (self.scratch.len() as u64, self.draws.len() as u64);
        let have_vertices = self.vertices.size() / vertex_size;
        let have_slots = self.slots.size() / self.stride;
        if need_vertices > have_vertices {
            (self.vertices, self.table_group) =
                Self::vertex_buffer(device, &self.table, need_vertices.next_power_of_two());
        }
        if need_slots > have_slots {
            (self.slots, self.group) =
                Self::slot_buffer(device, layout, need_slots.next_power_of_two(), self.stride);
        }
        let quads = self.draws.iter().map(|d| d.quads).max().unwrap_or(0);
        if quads > self.quads {
            self.quads = quads.next_power_of_two();
            self.indices = quad_indices(device, self.quads);
        }
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&self.scratch));
        queue.write_buffer(&self.slots, 0, &self.uniforms);
    }
}

/// Two triangles for each of `quads` quads, from vertex 0.
pub(super) fn quad_indices(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    let indices: Vec<u32> = (0..quads)
        .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
        .collect();
    wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("Shader pack entity indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        },
    )
}

/// Whether a run holds only geometry a gbuffers program can draw from the shared atlas: no
/// celestial body, cloud, beam, End portal, crack or energy swirl (overlay codes Horizon's entity
/// shader draws in its own way; entity.wgsl).
fn plain(run: &[Vertex]) -> bool {
    run.iter()
        .step_by(4)
        .all(|v| !matches!(v.layer & 0xffff, 2 | 4 | 6 | 9 | 10 | 11 | 12 | 34))
}

/// The programs of entity runs, in the order they draw.
pub const ENTITY: usize = 0;
pub const BLOCK: usize = 1;
pub const TRANSLUCENT: usize = 2;
pub const PARTICLE: usize = 3;
pub const PROGRAMS: usize = 4;

/// Which program draws `span`'s run, given which of them (or their fallbacks) the pack has;
/// `None`: Horizon draws it.
#[cfg(test)]
fn drawn_by_pack(span: &EntitySpan, drawn: [bool; PROGRAMS]) -> Option<usize> {
    program_of(span).filter(|&p| drawn[p])
}

/// The kind of program that draws `span`'s run, when a pack may.
pub fn program_of(span: &EntitySpan) -> Option<usize> {
    Some(if span.stage == render_stage("ENTITIES") {
        if span.translucent {
            TRANSLUCENT
        } else {
            ENTITY
        }
    } else if span.stage == render_stage("BLOCK_ENTITIES") {
        BLOCK
    } else if span.stage == render_stage("PARTICLES") {
        PARTICLE
    } else {
        return None;
    })
}

/// Each span with its vertex range: from its start to the next span's (or the end).
pub fn runs(
    len: usize,
    spans: &[EntitySpan],
) -> impl Iterator<Item = (EntitySpan, Range<usize>)> + '_ {
    spans.iter().enumerate().map(move |(i, s)| {
        let end = spans.get(i + 1).map_or(len, |n| n.start as usize);
        let start = (s.start as usize).min(len);
        (*s, start..end.clamp(start, len))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_split_the_list_between_pack_and_horizon() {
        let v = |layer| Vertex {
            layer,
            ..bytemuck::Zeroable::zeroed()
        };
        let span = |start, stage: &str, id| EntitySpan {
            start,
            stage: render_stage(stage),
            id,
            ..Default::default()
        };
        let spans = [
            span(2, "ENTITIES", 7),
            span(5, "PARTICLES", 0),
            span(6, "BLOCK_ENTITIES", 9),
        ];
        let ranges: Vec<_> = runs(10, &spans).map(|(_, r)| r).collect();
        assert_eq!(ranges, [2..5, 5..6, 6..10]);
        let all = [true; PROGRAMS];
        assert_eq!(drawn_by_pack(&spans[0], all), Some(ENTITY));
        assert_eq!(drawn_by_pack(&spans[1], all), Some(PARTICLE));
        assert_eq!(drawn_by_pack(&spans[1], [true, true, true, false]), None);
        assert_eq!(drawn_by_pack(&spans[2], [true, false, true, true]), None);
        assert_eq!(drawn_by_pack(&spans[2], all), Some(BLOCK));
        let shadow = EntitySpan {
            translucent: true,
            ..spans[0]
        };
        assert_eq!(drawn_by_pack(&shadow, all), Some(TRANSLUCENT));
        assert_eq!(drawn_by_pack(&span(0, "DESTROY", 0), all), None);
        // A beacon beam's quads stay Horizon's.
        assert!(plain(&[v(0), v(0), v(0), v(0), v(1)]));
        assert!(!plain(&[v(0), v(0), v(0), v(0), v(6)]));
        assert_eq!(entity_color(1)[0], 1.0);
        assert!((entity_color(1)[3] - (1.0 - 178.0 / 255.0)).abs() < 1e-6);
        assert_eq!(entity_color(0), [0.0; 4]);
        // The shadow pass: entities, the player alone, block entities, those that emit light.
        let all = Casters {
            entities: true,
            blocks: true,
            ..Default::default()
        };
        let player = EntitySpan {
            player: true,
            ..spans[0]
        };
        let lit = EntitySpan {
            emissive: true,
            ..spans[2]
        };
        assert!(all.cast(&spans[0], ENTITY) && all.cast(&spans[2], BLOCK));
        assert!(!all.cast(&spans[1], PARTICLE));
        let some = Casters {
            only_player: true,
            only_lit: true,
            ..all
        };
        assert!(!some.cast(&spans[0], ENTITY) && some.cast(&player, ENTITY));
        assert!(!some.cast(&spans[2], BLOCK) && some.cast(&lit, BLOCK));
    }
}
