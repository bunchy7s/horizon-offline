//! Two offscreen frames presented in one window, without game resources or player data.
//! `cargo run --profile fast -p horizon-client-render --example compositor -- --smoke`
use horizon_client_render::gfx::compositor::Compositor;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

struct Frames {
    window: Arc<Window>,
    compositor: Compositor,
    textures: [wgpu::Texture; 2],
}

impl Frames {
    fn new(event_loop: &ActiveEventLoop) -> horizon_client_render::Result<Self> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Compositor: red and blue views")
                    .with_inner_size(winit::dpi::PhysicalSize::new(640, 360)),
            )?,
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(window.clone()),
        ));
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
        let textures = [[0, 0, 255, 255], [255, 0, 0, 255]].map(|pixel| {
            let size = wgpu::Extent3d {
                width: 320,
                height: 360,
                depth_or_array_layers: 1,
            };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Offscreen view"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &pixel.repeat(320 * 360),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(320 * 4),
                    rows_per_image: Some(360),
                },
                size,
            );
            texture
        });
        let compositor = Compositor::new(
            surface,
            &adapter,
            &device,
            &queue,
            window.inner_size().into(),
        )?;
        Ok(Self {
            window,
            compositor,
            textures,
        })
    }
}

#[derive(Default)]
struct Demo {
    frames: Option<Frames>,
    error: Option<String>,
    presented: usize,
    smoke: bool,
}

impl ApplicationHandler for Demo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.frames.is_none() {
            match Frames::new(event_loop) {
                Ok(frames) => self.frames = Some(frames),
                Err(error) => {
                    self.error = Some(error.to_string());
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(f) = &mut self.frames else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => f.compositor.resize(size.width, size.height),
            // Uncovered pixels are black; resize smaller to exercise clipping.
            WindowEvent::RedrawRequested
                if f.compositor
                    .present(&[(&f.textures[0], [0, 0]), (&f.textures[1], [320, 0])]) =>
            {
                self.presented += 1;
                if self.smoke && self.presented == 10 {
                    f.compositor.set_vsync(false);
                    f.compositor.resize(0, 0);
                    assert!(!f.compositor.present(&[]));
                    f.compositor.resize(u32::MAX, u32::MAX);
                    assert!(!f.compositor.present(&[]));
                    f.compositor.resize(400, 240);
                    let _ = f
                        .window
                        .request_inner_size(winit::dpi::PhysicalSize::new(400, 240));
                }
                if self.smoke && self.presented == 20 {
                    println!("compositor: 20 frames, resize, suspension and clipping passed");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(f) = &self.frames {
            f.window.request_redraw();
        }
    }
}

fn main() -> horizon_client_render::Result<()> {
    let mut demo = Demo {
        smoke: std::env::args().any(|a| a == "--smoke"),
        ..Default::default()
    };
    EventLoop::new()?.run_app(&mut demo)?;
    if let Some(error) = demo.error {
        return Err(error.into());
    }
    if demo.smoke && demo.presented < 20 {
        return Err("smoke test closed early".into());
    }
    Ok(())
}
