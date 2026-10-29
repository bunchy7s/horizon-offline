//! The uniforms a shader pack's gbuffers programs read per draw (`HzDraw` at set 2, the
//! translator's `DRAW_UNIFORMS`): one slot per render stage of a uniform buffer, picked with a
//! dynamic offset at each draw.

use horizon_shaders::{RENDER_STAGES, render_stage};

/// Where a run of `Frame::entities` begins, and what it is (slice 7): the run lasts until the
/// next span. Recorded only while a shader pack draws entities, so Horizon's own frames carry
/// none; Horizon itself still draws the whole list at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntitySpan {
    /// The first vertex.
    pub start: u32,
    /// The render stage (5.4: `ENTITIES`, `BLOCK_ENTITIES`, `PARTICLES`, `DESTROY`), which picks
    /// the program.
    pub stage: u32,
    /// `entityId` for entities, `blockEntityId` for block entities (5.3), else 0.
    pub id: i32,
    /// An entity part that blends (a name tag, a shadow): `gbuffers_entities_translucent`.
    pub translucent: bool,
    /// Hidden from the camera (Horizon's entity culling, the player in first person): only the
    /// shadow pass draws it.
    pub hidden: bool,
    /// The local player (`shadowPlayer`).
    pub player: bool,
    /// A block entity whose block emits light (`shadowLightBlockEntities`).
    pub emissive: bool,
}

impl EntitySpan {
    /// Marks vertices `start..end` of `spans`' list (one entity) as hidden: the span covering
    /// `start` is split there, the spans inside are hidden, and the geometry after `end` goes
    /// on in the last span's kind. False when no span covers `start`.
    pub fn hide(spans: &mut Vec<EntitySpan>, start: usize, end: usize) -> bool {
        let Some(i) = spans.iter().rposition(|s| s.start as usize <= start) else {
            return false;
        };
        let after = spans[spans.len() - 1];
        if spans[i].start as usize != start {
            let split = EntitySpan {
                start: start as u32,
                ..spans[i]
            };
            spans.insert(i + 1, split);
        }
        for s in spans.iter_mut().skip(i) {
            if s.start as usize >= start {
                s.hidden = true;
            }
        }
        if end > start {
            spans.push(EntitySpan {
                start: end as u32,
                ..after
            });
        }
        true
    }
}

/// `HzDraw`'s size: `renderStage`, `entityId`, `blockEntityId`, a pad, `entityColor`.
const SIZE: u32 = 32;

pub(super) struct DrawUniforms {
    pub layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    stride: u32,
}

impl DrawUniforms {
    pub fn new(device: &wgpu::Device) -> Self {
        let stride = SIZE.max(device.limits().min_uniform_buffer_offset_alignment);
        let mut contents = vec![0u8; stride as usize * RENDER_STAGES.len()];
        for (i, slot) in contents.chunks_mut(stride as usize).enumerate() {
            slot[..4].copy_from_slice(&(i as i32).to_le_bytes());
        }
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("Shader pack draws"),
                contents: &contents,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack draws"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(u64::from(SIZE)),
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack draws"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(u64::from(SIZE)),
                }),
            }],
        });
        Self {
            layout,
            group,
            stride,
        }
    }

    /// Draws that follow read `renderStage` = `MC_RENDER_STAGE_<stage>`, other values zero.
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>, stage: &str) {
        pass.set_bind_group(2, &self.group, &[self.stride * render_stage(stage)]);
    }
}

#[cfg(test)]
mod tests {
    use super::EntitySpan;

    fn span(start: u32, id: i32) -> EntitySpan {
        EntitySpan {
            start,
            id,
            ..Default::default()
        }
    }

    #[test]
    fn hide_splits_the_span_around_one_entity() {
        // Two cows in one span: the second (vertices 8..16) is hidden.
        let mut spans = vec![span(0, 7)];
        assert!(EntitySpan::hide(&mut spans, 8, 16));
        let hidden = EntitySpan {
            hidden: true,
            ..span(8, 7)
        };
        assert_eq!(spans, [span(0, 7), hidden, span(16, 7)]);
        // An entity with its own span and a part after it (armour) is hidden whole.
        let mut spans = vec![span(0, 1), span(4, 2), span(6, 3)];
        assert!(EntitySpan::hide(&mut spans, 4, 10));
        assert!(!spans[0].hidden && spans[1].hidden && spans[2].hidden);
        assert_eq!(spans[3], span(10, 3));
        assert!(!EntitySpan::hide(&mut Vec::new(), 0, 4));
    }
}
