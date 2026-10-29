//! Rain and snow drawn by a shader pack's `gbuffers_weather` (spec 2.3): the game's weather
//! quads as entity vertices, their UV animation applied here rather than in Horizon's weather
//! shader, each kind textured from its own copy of its sprite so that the pack's `texture`
//! repeats it across the column as a standalone rain or snow texture would.

use crate::weather::{Time, Vertex as WeatherVertex};

use super::Vertex;
use super::pack_post::Weather;

/// What `gbuffers_weather` reads from a weather vertex: camera-relative position, the UV in
/// repeats of the sprite, colour (with vanilla's fade) and light. Weather has no normal; the
/// pack sees straight up.
pub fn inputs() -> horizon_shaders::GbuffersInputs {
    let declarations =
        "layout(location = 0) in vec3 hz_v_pos;\nlayout(location = 1) in vec2 hz_v_uv;\n\
         layout(location = 2) in uint hz_v_overlay;\nlayout(location = 3) in vec4 hz_v_color;\n\
         layout(location = 4) in uvec4 hz_v_light;\nlayout(location = 5) in vec4 hz_v_normal;\n"
            .to_owned();
    let body = "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    \
         hz_MultiTexCoord0 = vec4(hz_v_uv, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(float(hz_v_light.y), float(hz_v_light.x), 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n    \
         hz_Color = hz_v_color;\n    \
         hz_Normal = vec3(0.0, 1.0, 0.0);\n"
        .to_owned();
    let attributes = [
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaUV0", "hz_v_uv"),
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
        // Vanilla's weather drops texels below 0.1, as Horizon's own weather shader does.
        alpha_test: Some(horizon_shaders::AlphaTest {
            op: ">",
            reference: 0.1,
        }),
    }
}

/// The UV phase of `v` at `time`: `weather_phase` of entity.wgsl, on the CPU.
pub fn phase(v: &WeatherVertex, time: Time) -> [f32; 2] {
    let [ticks, fall_ticks, partial, _] = time.values;
    let a = v.animation;
    if v.snow == 0 {
        let t = ticks + a[1] + partial;
        return [0.0, (-t / 32.0 * a[0]) % 32.0];
    }
    let t = ticks + partial;
    let du = t * 0.01 * a[2];
    let dv = t * a[3] * 0.001;
    let base = |lo: u32, hi: u32| f64::from_bits(u64::from(lo) | u64::from(hi) << 32);
    let u = (base(v.bases[0], v.bases[1]) + f64::from(du)) as f32;
    let v_base = (base(v.bases[2], v.bases[3]) + f64::from(dv)) as f32;
    [u, -(fall_ticks + partial) / 512.0 + v_base]
}

/// One kind's sprite copied out of the shared atlas: the sprite's rectangle (bits) and its view.
type Sprite = Option<([u32; 4], wgpu::TextureView)>;

/// The pack's weather vertices and the rain and snow sprites, kept across frames.
pub struct PackWeather {
    vertices: wgpu::Buffer,
    capacity: usize,
    scratch: Vec<Vertex>,
    sprites: [Sprite; 2],
    /// Bumped when a sprite is copied again.
    generation: u64,
}

impl PackWeather {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            vertices: Self::buffer(device, 64),
            capacity: 64,
            scratch: Vec::new(),
            sprites: [None, None],
            generation: 0,
        }
    }

    fn buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack weather"),
            size: (capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// This frame's rain then snow (the game emits them in that order) as entity vertices, and
    /// the sprites they read. `None` when there is none.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        atlas: &wgpu::Texture,
        weather: &[WeatherVertex],
        time: Time,
        indices: &wgpu::Buffer,
    ) -> Option<Weather> {
        if weather.is_empty() {
            return None;
        }
        let rain = weather.iter().take_while(|v| v.snow == 0).count() as u32 / 4;
        let snow = weather.len() as u32 / 4 - rain;
        for (kind, first) in [(0, weather.first()), (1, weather.get(rain as usize * 4))] {
            let Some(first) = first.filter(|v| v.snow as usize == kind) else {
                continue;
            };
            let key = first.rect.map(f32::to_bits);
            if self.sprites[kind].as_ref().is_none_or(|s| s.0 != key) {
                let view = copy_sprite(device, encoder, atlas, first.rect);
                self.sprites[kind] = Some((key, view));
                self.generation += 1;
            }
        }
        self.scratch.clear();
        self.scratch.extend(weather.iter().map(|v| {
            let [du, dv] = phase(v, time);
            Vertex {
                pos: v.pos,
                uv: [v.uv[0] + du, v.uv[1] + dv],
                layer: 0,
                color: v.color,
                light: v.light,
                normal: [0; 4],

                glint_uv: [0.0, 0.0],
            }
        }));
        if self.scratch.len() > self.capacity {
            self.capacity = self.scratch.len().next_power_of_two();
            self.vertices = Self::buffer(device, self.capacity);
        }
        uploads.buffer(
            encoder,
            &self.vertices,
            0,
            bytemuck::cast_slice(&self.scratch),
        );
        let view = |k: usize| self.sprites[k].as_ref().map(|s| s.1.clone());
        Some(Weather {
            vertices: self.vertices.clone(),
            indices: indices.clone(),
            quads: [0..rain, rain..rain + snow],
            sprites: [view(0), view(1)],
            generation: self.generation,
        })
    }
}

/// A texture holding the atlas's texels under `rect` (normalised x, y, width, height), as
/// Horizon's weather shader reads them (`weather_texel`).
fn copy_sprite(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    atlas: &wgpu::Texture,
    rect: [f32; 4],
) -> wgpu::TextureView {
    let size = [atlas.width() as f32, atlas.height() as f32];
    let origin = [0, 1].map(|i| (rect[i] * size[i]).round() as u32);
    let extent = [0, 1].map(|i| ((rect[i + 2] * size[i]).round() as u32).max(1));
    let extent = [0, 1].map(|i| {
        extent[i].min(
            [atlas.width(), atlas.height()][i]
                .saturating_sub(origin[i])
                .max(1),
        )
    });
    let size = wgpu::Extent3d {
        width: extent[0],
        height: extent[1],
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Shader pack weather sprite"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: atlas.format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: atlas,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin[0],
                y: origin[1],
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        size,
    );
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::Parameters;

    /// The CPU phase matches the game's own weather columns (`Parameters::column`), which
    /// Horizon's weather shader is tested against.
    #[test]
    fn phase_matches_the_weather_columns() {
        for i in 0..512i32 {
            let x = i.wrapping_mul(45_238_971);
            let z = i.wrapping_mul(-418_711).wrapping_add(i32::MIN);
            let p = Parameters::new(x, z);
            let bases = p.snow_base.map(f64::to_bits);
            for ticks in [0, 511, 512, 131_071, 131_072, 1_000_003] {
                let partial = [0.0, 0.5, 0.999][i as usize % 3];
                for snow in [false, true] {
                    let column = p.column(snow, ticks, x, -64, 320, z, 0, partial);
                    let v = WeatherVertex {
                        pos: [0.0; 3],
                        uv: [0.0; 2],
                        rect: [0.0; 4],
                        animation: [
                            p.rain_speed,
                            p.rain_tick_offset,
                            p.snow_drift[0],
                            p.snow_drift[1],
                        ],
                        bases: [
                            bases[0] as u32,
                            (bases[0] >> 32) as u32,
                            bases[1] as u32,
                            (bases[1] >> 32) as u32,
                        ],
                        color: 0,
                        light: 0,
                        snow: u32::from(snow),
                    };
                    let [u, v] = phase(&v, Time::new(ticks, partial));
                    assert_eq!(
                        [u.to_bits(), v.to_bits()],
                        [column.u_offset.to_bits(), column.v_offset.to_bits()],
                        "x {x} z {z} ticks {ticks} snow {snow}"
                    );
                }
            }
        }
    }
}
