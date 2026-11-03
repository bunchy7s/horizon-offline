//! Stable entity material grouping with renderer-owned, reusable CPU scratch.
use super::Vertex;

/// Entity materials by a quad's overlay (`Vertex::layer`), in draw order:
/// ordinary entities, world text drawn with a depth bias (35,
/// `RenderPipelines.TEXT_POLYGON_OFFSET`), breaking cracks (11), translucent
/// breaking cracks (12), the boat water mask (13, `RenderPipelines.WATER_MASK`),
/// energy swirls (34, `RenderPipelines.ENERGY_SWIRL`), entity shadows (14,
/// `RenderPipelines.ENTITY_SHADOW`), translucent-layer particles (15, drawn
/// after translucent terrain: `SubmitNodeCollection.submitQuadParticleGroup`)
/// and translucent item quads (32, drawn without depth writes).
///
/// With improved transparency on, quads tagged `OIT_ENTITY` are a tenth
/// material at the end: the render types vanilla gives an `OitPipelineSet`
/// (translucent, emissive, eyes and breeze wind entity models). Off, the tag is
/// ignored and they stay in the first one, in their order.
const MATERIALS: usize = 10;
/// A bit of `Vertex::layer`, above the overlay (low 16 bits) and the HDR tier
/// (bits 16 to 23): the quad's render type has an `OitPipelineSet`
/// (`RenderTypes.entityTranslucent`, `entityTranslucentEmissive`, `eyes`,
/// `breezeWind`). Only the improved transparency path reads it.
pub const OIT_ENTITY: u32 = 1 << 24;
/// `RenderPipelines.DRAGON_RAYS`: untextured colour, additive with depth writes.
pub const DRAGON_RAYS_OVERLAY: u32 = 37;
/// `RenderPipelines.ENTITY_CUTOUT_DISSOLVE` (the dying dragon): a pixel is dropped where the
/// vertex alpha is below the mask's, whose atlas UV rides in `Vertex::glint_uv`; the rest is
/// drawn opaque. No overlay.
pub const DISSOLVE_OVERLAY: u32 = 38;
/// Tags quads as drawn by a render type vanilla gives an `OitPipelineSet`
/// (`OIT_ENTITY`): improved transparency draws them in its stages.
pub fn mark_oit(quads: &mut [Vertex]) {
    for vertex in quads {
        vertex.layer |= OIT_ENTITY;
    }
}
/// The overlay of depth-only quads that hide water (`RenderTypes.waterMask`).
pub const WATER_MASK_OVERLAY: u32 = 13;
/// The overlay of energy swirl quads (`RenderTypes.energySwirl`: the charged creeper's and the
/// powered wither's armour). The light bytes carry the sprite's origin and the colour bytes its
/// size (`breaking::stage_light`), the UV the scrolled position in the sprite.
pub const ENERGY_SWIRL_OVERLAY: u32 = 34;
/// The overlay of entity shadow quads (`RenderTypes.entityShadow`).
pub const SHADOW_OVERLAY: u32 = 14;
/// The overlay of translucent-layer particles (`SingleQuadParticle.Layer.translucent`).
pub const TRANSLUCENT_PARTICLE_OVERLAY: u32 = 15;
/// The overlay of item quads with translucent pixels (`Sheets.translucentItemSheet`,
/// `RenderPipelines.ITEM_TRANSLUCENT`); past the creeper swell range (16 to 31).
pub const TRANSLUCENT_ITEM_OVERLAY: u32 = 32;
/// A translucent item with an enchantment glint.
pub const TRANSLUCENT_GLINT_ITEM_OVERLAY: u32 = 33;
/// The overlay of text in `Font.DisplayMode.POLYGON_OFFSET` (text displays):
/// drawn like an ordinary entity, pulled toward the camera.
pub const TEXT_POLYGON_OFFSET_OVERLAY: u32 = 35;

/// The material of a quad by its overlay (`MATERIALS` order): the overlay is the layer's low 16
/// bits, as entity.wgsl reads it; HDR output's tier rides in the high ones.
fn material(layer: u32, oit: bool) -> usize {
    if oit && layer & OIT_ENTITY != 0 {
        return 9;
    }
    match layer & 0xffff {
        TEXT_POLYGON_OFFSET_OVERLAY => 1,
        11 => 2,
        12 => 3,
        WATER_MASK_OVERLAY => 4,
        ENERGY_SWIRL_OVERLAY => 5,
        SHADOW_OVERLAY => 6,
        TRANSLUCENT_PARTICLE_OVERLAY => 7,
        TRANSLUCENT_ITEM_OVERLAY | TRANSLUCENT_GLINT_ITEM_OVERLAY => 8,
        _ => 0,
    }
}

#[derive(Default)]
pub(super) struct EntityBatch {
    // Runs of consecutive quads of one material (first quad, quad count): a
    // model's quads share one, so whole runs are copied at once. Cleared each
    // frame, keeping their storage.
    runs: [Vec<(usize, usize)>; MATERIALS],
    pub(super) combined: Vec<Vertex>,
    /// Additive quads tagged `OIT_ENTITY` that the last `update` moved into the
    /// last material (improved transparency on): the dragon's eyes, which share
    /// the additive buffer with its rays.
    pub(super) moved_additive: u32,
}

impl EntityBatch {
    /// After `update`: quads only a native pack's shadow pass draws, at the buffer's end.
    pub(super) fn append_shadow_only(&mut self, shadow_only: &[Vertex]) {
        self.combined.extend_from_slice(shadow_only);
    }

    pub(super) fn update(
        &mut self,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
        oit: bool,
    ) -> [u32; MATERIALS] {
        let quads = entities.as_chunks::<4>().0;
        for runs in &mut self.runs {
            runs.clear();
        }
        let mut counts = [0usize; MATERIALS];
        let mut start = 0;
        while start < quads.len() {
            let kind = material(quads[start][0].layer, oit);
            let end = quads[start + 1..]
                .iter()
                .position(|quad| material(quad[0].layer, oit) != kind)
                .map_or(quads.len(), |n| start + 1 + n);
            self.runs[kind].push((start, end - start));
            counts[kind] += end - start;
            start = end;
        }
        let entity_vertices = quads.len() * 4;
        self.combined.clear();
        self.combined
            .reserve(entity_vertices + additive.len() + hand.len() + see_through.len());
        for runs in &self.runs {
            for &(first, count) in runs {
                self.combined
                    .extend_from_slice(&entities[first * 4..(first + count) * 4]);
            }
        }
        // `RenderTypes.eyes` is blended like the other entities, not added: with
        // improved transparency on, tagged additive quads join the last
        // material, drawn without the additive overlay and the baked shading.
        self.moved_additive = 0;
        if oit {
            for quad in additive.as_chunks::<4>().0 {
                if quad[0].layer & OIT_ENTITY != 0 {
                    self.combined.extend(quad.iter().map(|v| Vertex {
                        layer: v.layer & !0xffff,
                        normal: [0; 4],
                        ..*v
                    }));
                    counts[9] += 1;
                    self.moved_additive += 1;
                }
            }
        }
        if self.moved_additive == 0 {
            self.combined.extend_from_slice(additive);
        } else {
            for quad in additive.as_chunks::<4>().0 {
                if quad[0].layer & OIT_ENTITY == 0 {
                    self.combined.extend_from_slice(quad);
                }
            }
        }
        self.combined.extend_from_slice(hand);
        self.combined.extend_from_slice(see_through);
        counts.map(|count| count as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};

    // Retain the original implementation as the equivalence/benchmark oracle.
    fn baseline(
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) -> (Vec<Vertex>, [u32; MATERIALS]) {
        let mut combined = Vec::with_capacity(entities.len() + additive.len() + hand.len());
        let mut counts = [0; MATERIALS];
        for (i, layer) in [0, 35, 11, 12, 13, 34, 14, 15].into_iter().enumerate() {
            for quad in entities.as_chunks::<4>().0 {
                let material = match quad[0].layer {
                    35 => 35,
                    11 => 11,
                    12 => 12,
                    13 => 13,
                    34 => 34,
                    14 => 14,
                    15 => 15,
                    _ => 0,
                };
                if material == layer {
                    combined.extend_from_slice(quad);
                    counts[i] += 1;
                }
            }
        }
        combined.extend_from_slice(additive);
        combined.extend_from_slice(hand);
        (combined, counts)
    }

    fn geometry(quads: usize, mixed: bool) -> Vec<Vertex> {
        (0..quads * 4)
            .map(|index| Vertex {
                pos: [index as f32, 2.0, 3.0],
                uv: [index as f32, 5.0],
                layer: if mixed {
                    [7, 11, 14, 35, 12, 4, 13, 15, 0][index / 4 % 9]
                } else {
                    0
                },
                color: index as u32,
                light: (index * 3) as u32,
                normal: [1, 2, 3, 4],

                glint_uv: [0.0, 0.0],
            })
            .collect()
    }

    fn equivalent(
        batch: &mut EntityBatch,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) {
        let (expected, counts) = baseline(entities, additive, hand);
        assert_eq!(batch.update(entities, additive, hand, &[], false), counts);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&batch.combined),
            bytemuck::cast_slice::<_, u8>(&expected)
        );
    }

    #[test]
    fn a_tier_in_the_high_bits_keeps_the_material() {
        for layer in [0u32, 11, 12, 15, 32, 33, 35, OIT_ENTITY] {
            for oit in [false, true] {
                assert_eq!(material(layer | 4 << 16, oit), material(layer, oit));
            }
        }
    }

    #[test]
    fn oit_entity_quads_are_a_last_material_only_when_on() {
        let quad = |layer: u32, x: f32| {
            (0..4).map(move |_| Vertex {
                pos: [x, 0.0, 0.0],
                layer,
                ..geometry(1, false)[0]
            })
        };
        // Plain, tagged, plain, shadow, tagged with a hurt overlay.
        let entities: Vec<Vertex> = [
            (0, 0.0),
            (OIT_ENTITY | 3 << 16, 1.0),
            (1, 2.0),
            (SHADOW_OVERLAY, 3.0),
            (OIT_ENTITY | 1, 4.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let mut batch = EntityBatch::default();
        // Off: the tag changes nothing, the quads keep their place.
        let counts = batch.update(&entities, &[], &[], &[], false);
        assert_eq!(counts, [4, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        let xs: Vec<f32> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0, 4.0, 3.0]);
        // On: the tagged quads come last, in order.
        let counts = batch.update(&entities, &[], &[], &[], true);
        assert_eq!(counts, [2, 0, 0, 0, 0, 0, 1, 0, 0, 2]);
        let xs: Vec<f32> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 2.0, 3.0, 1.0, 4.0]);
    }

    #[test]
    fn tagged_additive_quads_join_the_last_material_only_when_on() {
        let quad = |layer: u32, x: f32| {
            (0..4).map(move |_| Vertex {
                pos: [x, 0.0, 0.0],
                layer,
                normal: [1, 2, 3, 4],
                ..geometry(1, false)[0]
            })
        };
        // The dragon's eyes (tagged, overlay 2) and a ray (plain).
        let additive: Vec<Vertex> = [(2 | OIT_ENTITY | 3 << 16, 1.0), (2, 2.0)]
            .into_iter()
            .flat_map(|(layer, x)| quad(layer, x))
            .collect();
        let entities: Vec<Vertex> = quad(0, 0.0).collect();
        let mut batch = EntityBatch::default();
        // Off: untouched.
        let counts = batch.update(&entities, &additive, &[], &[], false);
        assert_eq!((counts[9], batch.moved_additive), (0, 0));
        assert_eq!(batch.combined.len(), 12);
        assert_eq!(batch.combined[4].layer, 2 | OIT_ENTITY | 3 << 16);
        // On: the eyes are an entity quad without the additive overlay, the ray stays.
        let counts = batch.update(&entities, &additive, &[], &[], true);
        assert_eq!((counts[0], counts[9], batch.moved_additive), (1, 1, 1));
        let xs: Vec<f32> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0]);
        assert_eq!(batch.combined[4].layer, OIT_ENTITY | 3 << 16);
        assert_eq!(batch.combined[4].normal, [0; 4]);
        assert_eq!(batch.combined[8].layer, 2);
    }

    #[test]
    fn stable_material_order_tails_and_reuse() {
        let mut batch = EntityBatch::default();
        let mut entities = geometry(4096, true);
        // Only the first vertex determines a quad's material.
        entities[1].layer = 12;
        let additive = geometry(3, true);
        let hand = geometry(2, false);
        // Warm every bucket and the combined buffer to their maximum sizes.
        equivalent(&mut batch, &entities, &additive, &hand);
        let capacities = batch.runs.each_ref().map(|b| b.capacity());
        let capacity = batch.combined.capacity();
        let pointer = batch.combined.as_ptr();
        for length in [0, 1, 3, 4, 511, entities.len(), 17, 0, entities.len()] {
            equivalent(&mut batch, &entities[..length], &additive[..5], &hand[..3]);
            assert_eq!(batch.runs.each_ref().map(|b| b.capacity()), capacities);
            assert_eq!(batch.combined.capacity(), capacity);
            assert_eq!(batch.combined.as_ptr(), pointer);
        }
        equivalent(&mut batch, &[], &[], &[]);
        assert!(batch.combined.is_empty());
        // Grow again, then confirm empty/small frames retain the new storage.
        let larger = geometry(8192, true);
        equivalent(&mut batch, &larger, &additive, &hand);
        let capacities = batch.runs.each_ref().map(|b| b.capacity());
        let capacity = batch.combined.capacity();
        for entities in [&larger[..7], &larger[..], &[]] {
            equivalent(&mut batch, entities, &[], &[]);
            assert_eq!(batch.runs.each_ref().map(|b| b.capacity()), capacities);
            assert_eq!(batch.combined.capacity(), capacity);
        }
    }

    #[test]
    #[ignore = "manual production-helper microbenchmark"]
    fn benchmark_entity_batch() {
        let additive = geometry(16, false);
        let hand = geometry(8, false);
        for (name, mixed, quads) in [
            ("mixed", true, 65_536),
            ("single", false, 65_536),
            ("runs", true, 65_536),
            ("mixed", true, 262_144),
            ("single", false, 262_144),
        ] {
            let mut entities = geometry(quads, mixed);
            if name == "runs" {
                // Model/material runs: normal 16, glint 4, overlay 4, shadow 2.
                for (index, quad) in entities.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let layer = match index % 26 {
                        0..16 => 0,
                        16..20 => 11,
                        20..24 => 12,
                        _ => 4,
                    };
                    for vertex in quad {
                        vertex.layer = layer;
                    }
                }
            }
            let mut batch = EntityBatch::default();
            equivalent(&mut batch, &entities, &additive, &hand);
            for _ in 0..50 {
                black_box(baseline(black_box(&entities), &additive, &hand));
                black_box(batch.update(black_box(&entities), &additive, &hand, &[], false));
                black_box(&batch.combined);
            }
            let mut before = Vec::new();
            let mut after = Vec::new();
            for _ in 0..15 {
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(baseline(black_box(&entities), &additive, &hand));
                }
                before.push(start.elapsed().as_nanos() / 100);
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(batch.update(black_box(&entities), &additive, &hand, &[], false));
                    black_box(&batch.combined);
                }
                after.push(start.elapsed().as_nanos() / 100);
            }
            before.sort_unstable();
            after.sort_unstable();
            println!(
                "{name}: {quads} quads, baseline={} ns/frame optimized={} ns/frame speedup={:.2}x scratch={} bytes",
                before[7],
                after[7],
                before[7] as f64 / after[7] as f64,
                batch.combined.capacity() * std::mem::size_of::<Vertex>()
                    + batch
                        .runs
                        .iter()
                        .map(|b| b.capacity() * std::mem::size_of::<(usize, usize)>())
                        .sum::<usize>()
            );
        }
    }
}
