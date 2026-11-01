//! The debug options screen's renderer entries (26.3 `DebugRenderer`): the
//! water levels, heightmaps, collision boxes, light levels and sturdy faces
//! around the camera. Lines join the debug gizmos; fills and billboard texts
//! go with the entities' translucent geometry, as the test markers do.
//!
//! No-ops, for want of the data: `chunk_section_paths`,
//! `chunk_section_octree` and `chunk_section_visibility` (Horizon culls on
//! the GPU, with no section occlusion graph or octree),
//! `visualize_chunks_on_server` (the server's chunk statuses),
//! `visualize_sky_light_sections` (the light engine's section data) and
//! `visualize_entity_supporting_blocks` (`Entity.mainSupportingBlockPos`).
use super::Game;
use horizon_client::{
    block_entity_render::vertex_light,
    debug_chart::srgb_lerp,
    gizmos::{self as g, Gizmos},
    hud::HudBatch,
    hud_layout::abgr,
    mesher::Vertex,
    text::Text,
};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::{collision_context::CollisionContext, heightmap::HeightmapType};

/// `WaterDebugRenderer`'s fill: `ARGB.colorFromFloat(0.15F, 0, 1, 0)`.
const WATER_FILL: u32 = 0x2600_FF00;
/// `SolidFaceRenderer`'s fill (-2130771968).
const SOLID_FACE_FILL: u32 = 0x80FF_0000;
/// `TextGizmo.Style.DEFAULT_SCALE`.
const TEXT_SCALE: f32 = 0.32;
/// `HeightMapRenderer.BOX_HEIGHT`.
const BOX_HEIGHT: f64 = 0.09375;

/// `HeightMapRenderer.getColor`, for the client chunk's heightmaps.
fn heightmap_color(kind: HeightmapType) -> u32 {
    match kind {
        HeightmapType::WorldSurfaceWg => 0xFFFF_FF00,
        HeightmapType::OceanFloorWg => 0xFFFF_00FF,
        HeightmapType::WorldSurface => 0xFF00_B200,
        HeightmapType::OceanFloor => 0xFF00_007F,
        HeightmapType::MotionBlocking => 0xFF00_4C4C,
        HeightmapType::MotionBlockingNoLeaves => 0xFF00_7F7F,
    }
}

/// `BlockPos.betweenClosed` of the cube `radius` around `center`.
fn cube(center: [i32; 3], radius: i32) -> impl Iterator<Item = [i32; 3]> {
    let r = -radius..=radius;
    r.clone().flat_map(move |dy| {
        let r = r.clone();
        r.clone().flat_map(move |dz| {
            r.clone()
                .map(move |dx| [center[0] + dx, center[1] + dy, center[2] + dz])
        })
    })
}

impl Game<'_> {
    /// `CollisionBoxRenderer`: the block collisions within 6 of the camera
    /// entity's box, white.
    pub(super) fn debug_renderer_gizmos(&self, g: &mut Gizmos) {
        if !self.debug_entry("visualize_collision_boxes") {
            return;
        }
        let Some(p) = &self.player else {
            return;
        };
        let bounds = p.bounds().inflate(Vec3::new(6.0, 6.0, 6.0));
        let (lo, hi) = (bounds.min_position(), bounds.max_position());
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                    let Ok(shape) = self.collision.collision_shape_with_context(
                        &mut scene,
                        self.world.block(x, y, z),
                        BlockPos::new(x, y, z),
                        &CollisionContext::empty(false),
                    ) else {
                        continue;
                    };
                    let at = [f64::from(x), f64::from(y), f64::from(z)];
                    for b in shape.boxes() {
                        let (min, max) = (b.min_position(), b.max_position());
                        g.cuboid(
                            [at[0] + min.x, at[1] + min.y, at[2] + min.z],
                            [at[0] + max.x, at[1] + max.y, at[2] + max.z],
                            g::WHITE,
                            g::STROKE_WIDTH,
                        );
                    }
                }
            }
        }
    }

    /// The fill and text renderers' geometry for this frame.
    pub(super) fn debug_renderer_geometry(&mut self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let water = self.debug_entry("visualize_water_levels");
        let heightmap = self.debug_entry("visualize_heightmap");
        let block_light = self.debug_entry("visualize_block_light_levels");
        let sky_light = self.debug_entry("visualize_sky_light_levels");
        let solid = self.debug_entry("visualize_solid_faces");
        let Some(p) = &self.player else {
            return;
        };
        if !(water || heightmap || block_light || sky_light || solid) {
            return;
        }
        let feet = [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32);
        let eye = camera.to_array().map(|v| v.floor() as i32);
        let mut fills: Vec<([[f64; 3]; 4], u32)> = Vec::new();
        let mut texts: Vec<([f64; 3], String, u32)> = Vec::new();
        if water {
            self.water_level_shapes(feet, &mut fills, &mut texts);
        }
        if heightmap {
            self.heightmap_shapes(eye, &mut fills);
        }
        if block_light || sky_light {
            light_level_texts(self, eye, block_light, sky_light, &mut texts);
        }
        if solid {
            self.solid_face_shapes(eye, &mut fills);
        }
        let start = out.len();
        self.mark_translucent_entity(start, "");
        let light = vertex_light(0xFF);
        let mut white = HudBatch::new([1, 1]);
        for (quad, color) in fills {
            // A plain colour: the atlas's white pixel at the fill's colour
            // (the batch takes ABGR).
            white.vertices.clear();
            white.fill(&self.gui.atlas, 0.0, 0.0, 1.0, 1.0, abgr(color));
            let Some(v) = white.vertices.first().copied() else {
                continue;
            };
            for corner in quad {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: (glam::DVec3::from_array(corner) - camera)
                        .as_vec3()
                        .to_array(),
                    uv: v.uv,
                    layer: 0,
                    color: v.color,
                    light,
                    glint_uv: [0.0, 0.0],
                });
            }
        }
        // `Gizmos.billboardText`: centred, without a background.
        let mut patches = Vec::new();
        for (at, text, color) in texts {
            patches.extend(self.name_tag_quads(
                glam::DVec3::from_array(at) - camera,
                &[(Text::literal(text).with_color(color & 0xFF_FFFF), 0.0)],
                color >> 24,
                false,
                2,
                0,
                g::billboard_text_unit(TEXT_SCALE),
                out,
            ));
        }
        self.map_patches.extend(patches);
    }

    /// `WaterDebugRenderer`: each water block within 10 of the feet filled
    /// to its height, with its amount over it.
    fn water_level_shapes(
        &self,
        feet: [i32; 3],
        fills: &mut Vec<([[f64; 3]; 4], u32)>,
        texts: &mut Vec<([f64; 3], String, u32)>,
    ) {
        use horizon_client::blocks::Fluid;
        for pos in cube(feet, 10) {
            let f = self
                .res
                .facts()
                .get(self.world.block(pos[0], pos[1], pos[2]));
            if f.fluid != Fluid::Water {
                continue;
            }
            let Some((height, _)) = self.fluid_height_of(pos[0], pos[1], pos[2], None) else {
                continue;
            };
            let [x, y, z] = pos.map(f64::from);
            let top = y + f64::from(height);
            let (lo, hi) = (f64::from(0.01f32), f64::from(0.99f32));
            fills.extend(
                g::cuboid_fill_quads([x + lo, y + lo, z + lo], [x + hi, top, z + hi])
                    .map(|q| (q, WATER_FILL)),
            );
            // FluidState.getAmount: a source holds 8, a flowing one its level.
            texts.push((
                [x + 0.5, top, z + 0.5],
                f.fluid_level.to_string(),
                0xFF00_0000,
            ));
        }
    }

    /// `HeightMapRenderer`: the client chunk's heightmaps of the 5x5 chunks
    /// around the camera, a small box per column, stacked by type.
    fn heightmap_shapes(&self, eye: [i32; 3], fills: &mut Vec<([[f64; 3]; 4], u32)>) {
        let (cx, cz) = (eye[0] >> 4, eye[2] >> 4);
        for chunk_x in cx - 2..=cx + 2 {
            for chunk_z in cz - 2..=cz + 2 {
                if !self.world.chunks.contains_key(&(chunk_x, chunk_z)) {
                    continue;
                }
                for kind in HeightmapType::ALL
                    .into_iter()
                    .filter(|k| k.send_to_client())
                {
                    let color = heightmap_color(kind);
                    for x in chunk_x * 16..chunk_x * 16 + 16 {
                        for z in chunk_z * 16..chunk_z * 16 + 16 {
                            let height = f64::from(self.column_height(kind, x, z))
                                + f64::from(kind as u8) * BOX_HEIGHT;
                            let (x, z) = (f64::from(x), f64::from(z));
                            fills.extend(
                                g::cuboid_fill_quads(
                                    [x + 0.25, height, z + 0.25],
                                    [x + 0.75, height + BOX_HEIGHT, z + 0.75],
                                )
                                .map(|q| (q, color)),
                            );
                        }
                    }
                }
            }
        }
    }

    /// `SolidFaceRenderer`: each outline box of the blocks within 6 of the
    /// camera, grown by 0.002, with its sides that are sturdy filled.
    fn solid_face_shapes(&self, eye: [i32; 3], fills: &mut Vec<([[f64; 3]; 4], u32)>) {
        // In SolidFaceRenderer's order (WEST, SOUTH, EAST, NORTH, DOWN, UP):
        // the Direction ordinal and the side's quad in `cuboid_fill_quads`.
        const SIDES: [(u8, usize); 6] = [(4, 1), (3, 3), (5, 0), (2, 2), (0, 5), (1, 4)];
        for pos in cube(eye, 6) {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts().get(state);
            if &*f.name == "minecraft:air" || f.sturdy_full == 0 {
                continue;
            }
            let at = pos.map(f64::from);
            for b in f.outline.iter() {
                let min = [at[0] + b[0], at[1] + b[1], at[2] + b[2]].map(|v| v - 0.002);
                let max = [at[0] + b[3], at[1] + b[4], at[2] + b[5]].map(|v| v + 0.002);
                let quads = g::cuboid_fill_quads(min, max);
                for (direction, side) in SIDES {
                    if f.sturdy_full & (1 << direction) != 0 {
                        fills.push((quads[side], SOLID_FACE_FILL));
                    }
                }
            }
        }
    }
}

/// `LightDebugRenderer`: within 10 of the camera, the sky light where it
/// isn't full and the block light where there is some (the light engine's
/// section data, drawn with them in vanilla, Horizon doesn't keep).
fn light_level_texts(
    game: &Game<'_>,
    eye: [i32; 3],
    block: bool,
    sky: bool,
    texts: &mut Vec<([f64; 3], String, u32)>,
) {
    for pos in cube(eye, 10) {
        let light = game.world.light(pos[0], pos[1], pos[2]);
        let (sky_light, block_light) = (light >> 4, light & 15);
        let [x, y, z] = pos.map(f64::from);
        if sky_light != 15 && sky {
            let color = srgb_lerp(f32::from(sky_light) / 15.0, 0xFF00_00FF, 0xFF00_FFFF);
            texts.push(([x + 0.5, y + 0.25, z + 0.5], sky_light.to_string(), color));
        }
        if block && block_light != 0 {
            let color = srgb_lerp(f32::from(block_light) / 15.0, 0xFFAA_0000, 0xFFFF_FF00);
            texts.push(([x + 0.5, y + 0.5, z + 0.5], block_light.to_string(), color));
        }
    }
}
