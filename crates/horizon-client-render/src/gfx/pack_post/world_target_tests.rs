//! Original offscreen inputs exercise the production blit and post-processing output.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use std::io::{Cursor, Write};
use wgpu::TextureFormat as T;

#[derive(Clone, Copy, Debug)]
enum Change {
    Colour,
    Depth,
    Both,
    Contents,
    Resize,
}

#[test]
fn replacing_only_world_colour_at_the_same_size_reads_the_new_view() {
    replacement(Change::Colour, true);
}

#[test]
fn replacing_only_world_depth_at_the_same_size_reads_the_new_view() {
    replacement(Change::Depth, true);
}

#[test]
fn replacing_both_world_views_at_the_same_size_matches_fresh_bindings() {
    replacement(Change::Both, true);
}

#[test]
fn retained_world_views_read_updated_contents() {
    replacement(Change::Contents, true);
}

#[test]
fn resized_world_views_match_fresh_bindings() {
    replacement(Change::Resize, true);
}

#[test]
fn a_pack_without_final_reads_the_replaced_world_colour() {
    replacement(Change::Colour, false);
}

fn replacement(change: Change, final_shader: bool) {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    })) else {
        return;
    };
    eprintln!("world-target adapter: {:?}", adapter.get_info());
    let mut size = [4, 2];
    let mut colour = texture(&device, size, T::Rgba8Unorm);
    let mut depth = texture(&device, size, T::Depth32Float);
    clear(&device, &queue, &colour, &depth, [32, 64, 16, 255], 0.25);
    let mut post = pack(&device, &queue, &adapter, &colour, final_shader);
    let hud = HudBatch::new(size);
    let frame = frame(&hud);
    let initial = render(&device, &queue, &mut post, &colour, &depth, &frame, size);
    let expected = if final_shader {
        [32, 64, 191, 255]
    } else {
        [32, 64, 16, 255]
    };
    assert!(
        initial.iter().all(|pixel| *pixel == expected),
        "{initial:?}"
    );

    if matches!(change, Change::Resize) {
        size = [6, 3];
    }
    if matches!(change, Change::Colour | Change::Both | Change::Resize) {
        colour = texture(&device, size, T::Rgba8Unorm);
    }
    if matches!(change, Change::Depth | Change::Both | Change::Resize) {
        depth = texture(&device, size, T::Depth32Float);
    }
    let pixel = if matches!(change, Change::Depth) {
        [32, 64, 16, 255]
    } else {
        [160, 96, 48, 255]
    };
    let reverse_depth = if matches!(change, Change::Colour) {
        0.25
    } else {
        0.75
    };
    clear(&device, &queue, &colour, &depth, pixel, reverse_depth);
    let actual = render(&device, &queue, &mut post, &colour, &depth, &frame, size);
    let mut fresh = pack(&device, &queue, &adapter, &colour, final_shader);
    let reference = render(&device, &queue, &mut fresh, &colour, &depth, &frame, size);
    // Independently specified RG channels and GL depth (1 - reverse-Z), in UNORM8.
    let expected = if !final_shader {
        [160, 96, 48, 255]
    } else if matches!(change, Change::Colour) {
        [160, 96, 191, 255]
    } else if matches!(change, Change::Depth) {
        [32, 64, 64, 255]
    } else {
        [160, 96, 64, 255]
    };
    assert!(
        reference.iter().all(|pixel| *pixel == expected),
        "{reference:?}"
    );
    eprintln!("world-target {change:?}, final={final_shader}, size={size:?}: {actual:?}");
    assert_eq!(
        actual, reference,
        "current world inputs must match fresh bindings"
    );
}

fn texture(device: &wgpu::Device, [width, height]: [u32; 2], format: T) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Original world-target regression input"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn clear(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colour: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    pixel: [u8; 4],
    reverse_depth: f32,
) {
    let [r, g, b, a] = pixel.map(|n| f64::from(n) / 255.0);
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: colour,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(reverse_depth),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    queue.submit([encoder.finish()]);
}

fn pack(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    adapter: &wgpu::Adapter,
    lightmap: &wgpu::TextureView,
    final_shader: bool,
) -> PackPost {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("shaders/shaders.properties", options)
        .unwrap();
    zip.write_all(b"oldLighting=false\n").unwrap();
    if final_shader {
        for (name, source) in [
            (
                "final.vsh",
                "#version 120\nvarying vec2 uv;\nvoid main(){gl_Position=ftransform();uv=gl_MultiTexCoord0.xy;}",
            ),
            (
                "final.fsh",
                "#version 120\nvarying vec2 uv;uniform sampler2D colortex0;uniform sampler2D depthtex0;\nvoid main(){gl_FragColor=vec4(texture2D(colortex0,uv).rg,texture2D(depthtex0,uv).r,1.0);}",
            ),
        ] {
            zip.start_file(format!("shaders/{name}"), options).unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-world-inputs",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let macros = horizon_shaders::Macros::standard();
    let properties = horizon_shaders::Properties::parse("oldLighting=false", &macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let post = PackPost::new(
        device,
        queue,
        adapter,
        post,
        T::Rgba8Unorm,
        World {
            atlas: None,
            lightmap: lightmap.clone(),
            terrain: None,
            block_ids: &[],
            celestial: None,
            gui_size: [1, 1],
            clouds: None,
            weather: false,
            hand: false,
        },
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

#[allow(clippy::too_many_arguments)]
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    post: &mut PackPost,
    colour: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    frame: &Frame,
    size: [u32; 2],
) -> Vec<[u8; 4]> {
    let screen = texture(device, size, T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    post.run(
        device,
        queue,
        &mut encoder,
        colour,
        depth,
        &screen,
        frame,
        &Default::default(),
        size,
        size,
        &[],
        &mut |_, _, _, _| {},
    );
    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    read.slice(..)
        .get_mapped_range()
        .unwrap()
        .chunks_exact(stride as usize)
        .flat_map(|row| {
            row[..size[0] as usize * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
        })
        .collect()
}

fn frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        eye: glam::DVec3::ZERO,
        eye_rotation: [0.0; 2],
        first_person: true,
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        water_plane: None,
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_degrees: 85.0,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: true,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: crate::block_entity_render::StaticCacheFrameStats::default(),
        entity_motion: &[],
        colored_lights: &[],
        emissive_layers: &[],
        shadow_only: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        celestial: &[],
        stars: None,
        hand: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        menu_blur: None,
        chunk_fade: 0.0,
        post_effects: crate::post_chain::Request::none(),
    }
}
