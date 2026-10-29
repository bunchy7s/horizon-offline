//! Screenshots of a swapchain that cannot be copied from. OpenGL's surfaces
//! offer `RENDER_ATTACHMENT` only, so a frame whose pixels are read back
//! (the screenshot key, a world's icon) is drawn into a texture that can be,
//! then drawn onto the swapchain image as a fullscreen triangle.

/// The texture a frame to read back is drawn into, and the pass that puts it
/// on the swapchain afterwards.
#[derive(Default)]
pub struct Readback {
    texture: Option<wgpu::Texture>,
    blitter: Option<(wgpu::TextureFormat, wgpu::util::TextureBlitter)>,
    /// A frame drew into `texture` and still has to reach the swapchain.
    drawn: bool,
}

impl Readback {
    /// The texture this frame draws into instead of the swapchain image
    /// `surface`: `None` unless the frame is read back (`screenshot`) and the
    /// image cannot be copied from.
    pub fn target(
        &mut self,
        device: &wgpu::Device,
        surface: &wgpu::Texture,
        screenshot: bool,
    ) -> Option<wgpu::Texture> {
        if !screenshot || surface.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let format = surface.format().remove_srgb_suffix();
        if self
            .texture
            .as_ref()
            .is_none_or(|t| t.size() != surface.size() || t.format() != format)
        {
            self.texture = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Horizon screenshot frame"),
                size: surface.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }));
        }
        self.drawn = true;
        self.texture.clone()
    }

    /// Draws the frame `target` gave onto the swapchain image `surface`; nothing
    /// when this frame drew straight into it.
    pub fn present(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        surface: &wgpu::Texture,
    ) {
        if !std::mem::take(&mut self.drawn) {
            return;
        }
        let Some(texture) = &self.texture else {
            return;
        };
        // The frame's passes write the swapchain through its non-sRGB view
        // (`view_formats`), so the copy goes there unconverted too.
        let format = surface.format().remove_srgb_suffix();
        if self.blitter.as_ref().is_none_or(|b| b.0 != format) {
            self.blitter = Some((format, wgpu::util::TextureBlitter::new(device, format)));
        }
        let target = surface.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        });
        let source = texture.create_view(&Default::default());
        if let Some((_, blitter)) = &self.blitter {
            blitter.copy(device, encoder, &source, &target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn texture(device: &wgpu::Device, usage: wgpu::TextureUsages) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 70,
                height: 3,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage,
            view_formats: &[],
        })
    }

    fn clear(encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, color: wgpu::Color) {
        let view = texture.create_view(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }

    fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        buffer.get_mapped_range(..).unwrap().to_vec()
    }

    /// The last pixel of the last row of a 70x3 frame read back by `copy_frame`.
    fn last_pixel(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
        let pixels = read(device, buffer);
        let row = 70usize * 4;
        let last = row.next_multiple_of(256) * 2 + row - 4;
        pixels[last..last + 4].to_vec()
    }

    /// The renderer's screenshot path on a surface like OpenGL's (#3510): the
    /// frame is drawn into `target`, read back with `copy_frame` and drawn on
    /// the swapchain, with no validation error (the copy used to need the
    /// swapchain's missing `COPY_SRC`), and the swapchain shows the frame.
    #[test]
    fn screenshot_of_a_surface_without_copy_src() {
        let Some((device, queue)) = device() else {
            return;
        };
        // Sampled only so the test can look at what reached it.
        let surface = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let check = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let blitter = wgpu::util::TextureBlitter::new(&device, wgpu::TextureFormat::Bgra8Unorm);
        let mut readback = Readback::default();
        let colors = [
            (1.0, [0, 0, 255, 255]),
            // A second screenshot reuses the texture.
            (0.0, [255, 0, 0, 255]),
        ];
        for (r, bgra) in colors {
            let color = wgpu::Color {
                r,
                g: 0.0,
                b: 1.0 - r,
                a: 1.0,
            };
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut encoder = device.create_command_encoder(&Default::default());
            let frame = readback
                .target(&device, &surface, true)
                .unwrap_or_else(|| surface.clone());
            clear(&mut encoder, &frame, color);
            let buffer = crate::gfx::copy_frame(&device, &mut encoder, &frame, [70, 3]);
            readback.present(&device, &mut encoder, &surface);
            queue.submit([encoder.finish()]);
            if let Some(error) = pollster::block_on(scope.pop()) {
                panic!("{error}");
            }
            assert_eq!(last_pixel(&device, &buffer), bgra);
            let mut encoder = device.create_command_encoder(&Default::default());
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            blitter.copy(&device, &mut encoder, &view(&surface), &view(&check));
            let buffer = crate::gfx::copy_frame(&device, &mut encoder, &check, [70, 3]);
            queue.submit([encoder.finish()]);
            assert_eq!(last_pixel(&device, &buffer), bgra);
        }
        assert!(!readback.drawn);
    }

    /// A surface that can be copied from, or a frame nobody reads back,
    /// draws straight into the swapchain image.
    #[test]
    fn frames_draw_straight_to_a_readable_surface() {
        let Some((device, _)) = device() else {
            return;
        };
        let mut readback = Readback::default();
        let readable = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        assert!(readback.target(&device, &readable, true).is_none());
        let surface = texture(&device, wgpu::TextureUsages::RENDER_ATTACHMENT);
        assert!(readback.target(&device, &surface, false).is_none());
        assert!(readback.texture.is_none() && !readback.drawn);
    }
}
