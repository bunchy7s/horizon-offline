//! The colour of block light for Horizon's shader packs (`[[lights]]`, docs/api/shaders.md):
//! a grid of cells around the camera, each holding the colour of the light sources that
//! reach it, averaged by how near each is, which the material stage reads through the
//! prelude's `block_light_color`. Block light keeps its own brightness (the light engine's,
//! which walls stop): the grid only tints it, so it needs no occlusion of its own.
//!
//! The grid is rebuilt on the CPU when the camera moves on to another 16-block step or the
//! light sources change, from the sources the section meshes list (`SectionMesh::lights`).

use crate::fast_hash::FxHashMap;
use glam::{DVec3, Vec3};

use super::SectionKey;

/// Cells along each side of the grid.
pub const SIZE: u32 = 64;
/// The side of a cell, in blocks: the grid spans 128 blocks.
pub const CELL: f32 = 2.0;
/// The grid moves in steps of this many blocks.
const STEP: i32 = 16;
/// The most sources one rebuild splats, the nearest first (a lava lake lists thousands).
const MAX_SOURCES: usize = 2048;
/// `[[lights]]` colours go up to 4; the cells keep them over 4 in 8 bits.
const COLOR_RANGE: f32 = 4.0;

/// The light a block state gives off: its `[[lights]]` colour and its level, 1 to 15.
pub type LightTable = FxHashMap<u32, ([f32; 3], u8)>;

pub struct BlockLight {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// `block_light_grid`: the grid's corner, camera-relative, and the side of a cell.
    pub grid: wgpu::Buffer,
    table: LightTable,
    /// The world corner and the sources' revision the texture was last built for.
    built: Option<([i32; 3], u64)>,
}

impl BlockLight {
    pub fn new(device: &wgpu::Device, table: LightTable) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Native shader pack block light colours"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let grid = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native shader pack block light grid"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            view: texture.create_view(&Default::default()),
            texture,
            grid,
            table,
            built: None,
        }
    }

    /// Places the grid around `camera`, rebuilding it when it moved or `revision` (of
    /// `sections`' sources) changed.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera: DVec3,
        sections: &FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
        revision: u64,
    ) {
        let half = (SIZE as f32 * CELL / 2.0) as f64;
        let origin = ((camera - half) / f64::from(STEP))
            .floor()
            .as_ivec3()
            .to_array()
            .map(|v| v * STEP);
        let corner = (glam::IVec3::from(origin).as_dvec3() - camera).as_vec3();
        queue.write_buffer(
            &self.grid,
            0,
            bytemuck::cast_slice(&[corner.x, corner.y, corner.z, CELL]),
        );
        if self.built == Some((origin, revision)) {
            return;
        }
        self.built = Some((origin, revision));
        let cells = build(&self.table, origin, camera, sections);
        queue.write_texture(
            self.texture.as_image_copy(),
            &cells,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
            self.texture.size(),
        );
    }
}

/// The cells of the grid with its corner at `origin`: RGBA8, x fastest, then y, then z.
fn build(
    table: &LightTable,
    origin: [i32; 3],
    camera: DVec3,
    sections: &FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
) -> Vec<u8> {
    let n = SIZE as usize;
    let mut cells = vec![0u8; n * n * n * 4];
    if table.is_empty() {
        return cells;
    }
    let span = SIZE as i32 * CELL as i32;
    // Sections the grid or the light reaching into it (15 blocks) touches.
    let lo = origin.map(|v| (v - 15).div_euclid(16));
    let hi = origin.map(|v| (v + span + 15).div_euclid(16));
    let mut sources: Vec<(Vec3, [f32; 3], f32, f64)> = Vec::new();
    for (key, lights) in sections {
        let key = [key.0, key.1 + crate::chunk::MIN_Y.div_euclid(16), key.2];
        if (0..3).any(|i| key[i] < lo[i] || key[i] > hi[i]) {
            continue;
        }
        for &(pos, state) in lights {
            let Some(&(color, level)) = table.get(&state) else {
                continue;
            };
            let center = glam::IVec3::from(pos).as_dvec3() + 0.5;
            let local = (center - glam::IVec3::from(origin).as_dvec3()).as_vec3();
            sources.push((
                local,
                color,
                f32::from(level),
                center.distance_squared(camera),
            ));
        }
    }
    if sources.len() > MAX_SOURCES {
        sources.select_nth_unstable_by(MAX_SOURCES, |a, b| a.3.total_cmp(&b.3));
        sources.truncate(MAX_SOURCES);
    }
    let mut sum = vec![[0.0f32; 4]; n * n * n];
    for (at, color, reach, _) in sources {
        let first = ((at - reach) / CELL).floor().max(Vec3::ZERO).as_ivec3();
        let last = ((at + reach) / CELL)
            .ceil()
            .min(Vec3::splat(SIZE as f32 - 1.0))
            .as_ivec3();
        for z in first.z..=last.z {
            for y in first.y..=last.y {
                for x in first.x..=last.x {
                    let cell = (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * CELL;
                    let d = cell.distance(at);
                    if d >= reach {
                        continue;
                    }
                    // As block light falls off: strongest at the source, gone at its level.
                    let w = (1.0 - d / reach).powi(2);
                    let s = &mut sum[x as usize + n * (y as usize + n * z as usize)];
                    s[0] += color[0] * w;
                    s[1] += color[1] * w;
                    s[2] += color[2] * w;
                    s[3] += w;
                }
            }
        }
    }
    for (cell, s) in cells.as_chunks_mut::<4>().0.iter_mut().zip(&sum) {
        if s[3] <= 0.0 {
            continue;
        }
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        for i in 0..3 {
            cell[i] = byte(s[i] / s[3] / COLOR_RANGE);
        }
        cell[3] = byte(s[3]);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_tints_the_cells_it_reaches_with_its_colour() {
        let mut table = LightTable::default();
        table.insert(7, ([2.0, 1.0, 0.0], 14));
        let mut sections = FxHashMap::default();
        // A torch at world (8, 64, 8): section y = (64 - MIN_Y) / 16.
        let sy = (64 - crate::chunk::MIN_Y) / 16;
        sections.insert((0, sy, 0), vec![([8, 64, 8], 7)]);
        let camera = DVec3::new(8.0, 64.0, 8.0);
        let origin = [-64, 0, -64];
        let cells = build(&table, origin, camera, &sections);
        let n = SIZE as usize;
        let at = |p: [i32; 3]| {
            let c = p
                .iter()
                .zip(origin)
                .map(|(v, o)| ((v - o) as f32 / CELL) as usize)
                .collect::<Vec<_>>();
            let i = (c[0] + n * (c[1] + n * c[2])) * 4;
            [cells[i], cells[i + 1], cells[i + 2], cells[i + 3]]
        };
        let near = at([8, 64, 8]);
        // The colour over 4, whatever the weight: (2, 1, 0) / 4.
        assert_eq!(&near[..3], &[128, 64, 0]);
        assert!(near[3] > 150);
        assert_eq!(at([8, 64, 40]), [0; 4], "past its level, nothing");
        assert!(
            build(&LightTable::default(), origin, camera, &sections)
                .iter()
                .all(|&b| b == 0)
        );
    }
}
