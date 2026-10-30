//! `PistonHeadRenderer`: while a piston moves them, blocks are
//! `minecraft:moving_piston` block entities (drawn as nothing by the chunk
//! mesher); this draws each moved block at its interpolated offset.
use super::Game;
use horizon_client::mesher::Vertex;
use horizon_world::{piston_shape::MovingPistonState, registry::BlockStateId};

impl Game<'_> {
    /// `PistonHeadRenderer.extractRenderState` and `submit`.
    pub(super) fn moving_piston_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let partial = alpha as f32;
        let live = self
            .world
            .moving_pistons
            .iter()
            .map(|&(p, s)| (p, s, false));
        // A piston that just finished bridges the frames until the chunk mesh
        // shows the block it placed; drawn over that mesh, it z-fights (#3002).
        let finished = self
            .world
            .moving_piston_fallback
            .iter()
            .filter(|(p, _)| self.section_mesh_pending(*p))
            .map(|&(p, s)| (p, s, true));
        let states: Vec<_> = live.chain(finished).collect();
        for (p, piston, placed) in states {
            let Some((block, base)) = self.moving_piston_blocks(piston, partial) else {
                continue;
            };
            let step = piston.movement_direction().normal();
            let at = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            // `createMovingBlock`'s block position (seed, tint and light): the
            // cell the block moves from. A finished one stands in for the
            // chunk mesh's block at `p`, so it is drawn as that block.
            let from = if placed {
                at
            } else {
                horizon_core::position::BlockPos::new(
                    p[0] - step[0],
                    p[1] - step[1],
                    p[2] - step[2],
                )
            };
            let offset = piston.interpolated_offset(partial);
            let corner = glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
            let moved = corner + glam::Vec3::from_array(offset).as_dvec3();
            self.moving_block_vertices(block, from, moved - camera, out);
            if let Some(base) = base {
                self.moving_block_vertices(base, at, corner - camera, out);
            }
        }
    }
    /// Whether the chunk mesh doesn't show the blocks at `p` yet (edited,
    /// meshing, or waiting for its upload).
    pub(super) fn section_mesh_pending(&self, p: [i32; 3]) -> bool {
        let Some(y) = p[1].checked_sub(horizon_client::chunk::MIN_Y) else {
            return false;
        };
        let section = (p[0] >> 4, y.div_euclid(16), p[2] >> 4);
        self.world.dirty_sections.contains(&section)
            || self.streaming.is_meshing(section)
            || self.pending_uploads.iter().any(|(key, _)| *key == section)
    }
    /// The moved block and, for a retracting piston, its extended base.
    fn moving_piston_blocks(
        &self,
        piston: MovingPistonState,
        partial: f32,
    ) -> Option<(BlockStateId, Option<BlockStateId>)> {
        let facts = self.res.facts();
        if facts.is_air(piston.moved_state) {
            return None;
        }
        let progress = piston.interpolated_progress(partial);
        let name = &*facts.get(piston.moved_state).name;
        // Vanilla tests `progress <= 4.0`: a moving head is always drawn so.
        if name == "minecraft:piston_head" {
            let short = if progress <= 0.5 { "true" } else { "false" };
            return Some((
                self.with_props(piston.moved_state, &[("short", short)])?,
                None,
            ));
        }
        if piston.source && !piston.extending {
            let registry = &self.pack.assets.registry;
            let facing = registry
                .state(piston.moved_state)?
                .properties
                .get("facing")?
                .clone();
            let kind = if name == "minecraft:sticky_piston" {
                "sticky"
            } else {
                "normal"
            };
            let short = if progress >= 0.5 { "true" } else { "false" };
            let head = self.with_props(
                registry.default_state("minecraft:piston_head")?,
                &[("type", kind), ("facing", &facing), ("short", short)],
            )?;
            let base = self.with_props(piston.moved_state, &[("extended", "true")])?;
            return Some((head, Some(base)));
        }
        Some((piston.moved_state, None))
    }
    /// One `MovingBlockRenderState`: the baked model at `at` (camera-relative),
    /// seeded by its block position `seed`, tinted per face with the block
    /// colours there and lit as `ModelBlockRenderer.tesselateFlat` lights a
    /// lone block at `seed`: each face from the cell it looks at. Not from
    /// the moving piston's own cell, which is dark once the block it placed
    /// stands there (#3002).
    fn moving_block_vertices(
        &self,
        state: BlockStateId,
        seed: horizon_core::position::BlockPos,
        at: glam::DVec3,
        out: &mut Vec<Vertex>,
    ) {
        let chunks = &self.world.chunks;
        let (cx, cz) = (seed.x >> 4, seed.z >> 4);
        let region: [[Option<&horizon_client::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        let tint = self.pack.context.tables.block_tints(
            state,
            [seed.x, seed.y, seed.z],
            &region,
            self.res.biomes(),
            self.streaming.biome_zoom,
            self.streaming.biome_blend,
        );
        let tables = &self.pack.context.tables;
        let light = |quad: &horizon_assets::bake::ModelQuad| {
            let coords = tables.flat_light(state, [seed.x, seed.y, seed.z], quad, |p| {
                self.world.light(p[0], p[1], p[2])
            });
            // Light coords (block << 4 | sky << 20) as the vertex packs them.
            (coords >> 16 & 0xF0) | ((coords & 0xF0) << 8)
        };
        self.tinted_block_model_vertices(
            state,
            horizon_core::random::position_seed(seed),
            glam::Mat4::from_translation(at.as_vec3()),
            light,
            false,
            tint,
            out,
        );
    }
}
