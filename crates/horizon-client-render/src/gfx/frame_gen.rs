//! Frame generation (a Horizon setting, off by default): FSR 3.1's frame interpolation
//! makes a frame halfway between each two real ones. The world, without the hand and
//! HUD, is kept at the screen's size each frame; optical flow and the interpolation run
//! on it, the hand and HUD are drawn over the made frame as over the real one, and a
//! present thread shows the made frame, then the real one half a real frame later.
//!
//! The renderer draws into frames of its own while this is on; the present thread
//! copies each to the swapchain, which it alone acquires and presents.

mod interpolation;
mod motion;
mod optical_flow;

use glam::{DMat4, DVec3, Mat4};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// Frames to draw into: a pair being drawn, a pair waiting and a pair being shown.
const FRAMES: usize = 6;

/// `HORIZON_FRAMEGEN_DEBUG=generated`: show only made frames (AMD's debug view), so a
/// screenshot or an offscreen frame is the made one; `motion`: the same, each made frame
/// showing what moved on its own in blue (its motion vectors in red and green).
pub(super) fn generated_only() -> bool {
    debug().is_some()
}

fn debug() -> Option<&'static str> {
    static DEBUG: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DEBUG
        .get_or_init(|| {
            std::env::var("HORIZON_FRAMEGEN_DEBUG")
                .ok()
                .filter(|v| v == "generated" || v == "motion")
        })
        .as_deref()
}

pub struct FrameGen {
    pub on: bool,
    /// The present thread failed to show a frame (or died): until the renderer has
    /// acquired a frame itself, the normal path runs and handles the surface.
    pub paused: bool,
    gpu: Option<Gpu>,
    presenter: Option<Presenter>,
    /// When the last real frame was handed over, and the time between real frames.
    last: Option<Instant>,
    interval: Duration,
    /// The last frame's unjittered `view_proj` and camera.
    previous: Option<(DMat4, DVec3)>,
    /// The last frame's hand vertices and the bob and view they were drawn with.
    hand: Vec<crate::mesher::Vertex>,
    hand_view: Mat4,
    /// Offscreen, the frame the made frame is drawn into.
    offscreen_generated: Option<wgpu::Texture>,
    /// Motion vectors for what moves on its own, made on the first frame.
    motion: Option<motion::ObjectMotion>,
    /// Real frames and made ones handed to the screen since `shown_since`, and the share
    /// of real frames that had a made frame over the last half second (the F3 screen's),
    /// `None` while off.
    real: u32,
    made: u32,
    shown_since: Instant,
    pub made_share: Option<f32>,
}

/// What runs on the GPU for one screen size.
struct Gpu {
    size: [u32; 2],
    flow: optical_flow::OpticalFlow,
    interpolation: interpolation::Interpolation,
    /// This frame's world at the screen's size, and bindings that copy it or the
    /// interpolation's output (`Post::finish_with`).
    world: wgpu::TextureView,
    world_group: wgpu::BindGroup,
    output_group: wgpu::BindGroup,
    /// The next interpolation has no previous frame.
    reset: bool,
}

/// This frame's camera, for the interpolation.
pub struct Camera {
    /// The world's projection this frame (with TAA's jitter) and without the jitter.
    pub jittered: Mat4,
    pub view_proj: Mat4,
    pub depth: wgpu::TextureView,
    pub render_size: [u32; 2],
    pub jitter: [f32; 2],
    /// This frame's unjittered clip space to the previous frame's, `None` without one.
    pub reproject: Option<Mat4>,
    pub near: f32,
    pub far: f32,
    pub fov_y: f32,
}

impl FrameGen {
    pub fn new() -> Self {
        Self {
            on: false,
            paused: false,
            gpu: None,
            presenter: None,
            last: None,
            interval: Duration::from_millis(16),
            previous: None,
            hand: Vec::new(),
            hand_view: Mat4::IDENTITY,
            offscreen_generated: None,
            motion: None,
            real: 0,
            made: 0,
            shown_since: Instant::now(),
            made_share: None,
        }
    }

    /// Ends frame generation's frames and present thread: before the surface is
    /// reconfigured or the renderer acquires from it itself.
    pub fn stop(&mut self) {
        self.presenter = None;
        self.made_share = None;
        (self.real, self.made) = (0, 0);
        self.last = None;
        self.previous = None;
        self.hand.clear();
        if let Some(motion) = &mut self.motion {
            motion.reset();
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.reset = true;
        }
    }

    /// The hand for the made frame, appended to `out`: each vertex halfway between where
    /// it was on screen last frame and where it is now (the hand swings and bobs on its
    /// own, so the world's interpolation can't move it), in this frame's space
    /// (`view`: this frame's bob and view). This frame's hand where the two differ in
    /// shape (another item, the first frame).
    pub fn halfway_hand(
        &mut self,
        hand: &[crate::mesher::Vertex],
        view: Mat4,
        out: &mut Vec<crate::mesher::Vertex>,
    ) {
        let same = self.hand.len() == hand.len()
            && self
                .hand
                .iter()
                .zip(hand)
                .all(|(a, b)| a.layer == b.layer && a.uv == b.uv);
        if same {
            let back = view.inverse();
            out.extend(self.hand.iter().zip(hand).map(|(before, now)| {
                let then = self.hand_view.transform_point3(before.pos.into());
                let mid = (then + view.transform_point3(now.pos.into())) * 0.5;
                crate::mesher::Vertex {
                    pos: back.transform_point3(mid).into(),
                    ..*now
                }
            }));
        } else {
            out.extend_from_slice(hand);
        }
        self.hand.clear();
        self.hand.extend_from_slice(hand);
        self.hand_view = view;
    }

    /// This frame's entity vertices (relative to `camera`) and the entity ids of their
    /// spans (`Frame::entity_motion`), for their own motion vectors; `atlas`: the
    /// entities' textures. Before `generate`, every frame.
    pub fn track_entities(
        &mut self,
        device: &wgpu::Device,
        atlas: &wgpu::TextureView,
        entities: &[crate::mesher::Vertex],
        spans: &[(u32, u64)],
        camera: DVec3,
    ) {
        self.motion
            .get_or_insert_with(|| motion::ObjectMotion::new(device, atlas.clone()))
            .track(entities, spans, camera);
    }

    /// This frame's unjittered clip space to the last frame's, from their `view_proj`s
    /// (relative to `camera`); `None` on the first frame.
    pub fn camera_motion(&mut self, view_proj: Mat4, camera: DVec3) -> Option<Mat4> {
        let current = view_proj.as_dmat4();
        let previous = self.previous.replace((current, camera));
        previous.map(|(p, pc)| super::world_post::reprojection(p, pc, current, camera).as_mat4())
    }

    /// Frames to draw the real and the made frame into, from the present thread's
    /// (started for this `config`'s surface) or, offscreen, the offscreen target and
    /// one of its own. `None` once the present thread failed or died: it is stopped and
    /// frame generation `paused`.
    pub fn frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: Option<&Arc<wgpu::Surface<'static>>>,
        offscreen: Option<&wgpu::Texture>,
        window: &Arc<winit::window::Window>,
        config: &wgpu::SurfaceConfiguration,
    ) -> Option<(wgpu::Texture, wgpu::Texture)> {
        let Some(surface) = surface else {
            let real = offscreen.cloned().unwrap_or_else(|| frame(device, config));
            if self
                .offscreen_generated
                .as_ref()
                .is_none_or(|t| t.size() != real.size())
            {
                self.offscreen_generated = Some(frame(device, config));
            }
            let generated = self.offscreen_generated.clone().expect("made above");
            return Some((real, generated));
        };
        if self.presenter.as_ref().is_some_and(|p| p.config != *config) {
            self.presenter = None;
        }
        let presenter = self.presenter.get_or_insert_with(|| {
            Presenter::start(device, queue, surface.clone(), window.clone(), config)
        });
        // A closed channel: the thread died.
        let pair = if presenter.failed() {
            None
        } else {
            presenter.free.recv().ok().zip(presenter.free.recv().ok())
        };
        if pair.is_none() {
            self.stop();
            self.paused = true;
        }
        pair
    }

    /// The world's target at the screen's size this frame, where `Post::finish` draws.
    pub fn world(
        &mut self,
        device: &wgpu::Device,
        post: &super::world_post::Post,
        config: &wgpu::SurfaceConfiguration,
    ) -> wgpu::TextureView {
        let size = [config.width, config.height];
        if self.gpu.as_ref().is_none_or(|g| g.size != size) {
            let world = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Frame generation world"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: config.format.remove_srgb_suffix(),
                    // COPY_SRC: the interpolation keeps it as the previous frame.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let interpolation = interpolation::Interpolation::new(device, size);
            self.gpu = Some(Gpu {
                size,
                flow: optical_flow::OpticalFlow::new(device, size),
                world_group: post.bind(device, &world),
                output_group: post.bind(device, interpolation.output()),
                interpolation,
                world,
                reset: true,
            });
        }
        self.gpu.as_ref().expect("made above").world.clone()
    }

    /// Copy the world (`world`) onto `screen` for the real frame, and draw the made
    /// frame's world onto `generated`. Returns false when there is no made frame (the
    /// first after a reset): `generated` then holds this frame's world too.
    #[allow(clippy::too_many_arguments)]
    pub fn generate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        post: &super::world_post::Post,
        screen: &wgpu::TextureView,
        generated: &wgpu::TextureView,
        camera: &Camera,
    ) -> bool {
        let gpu = self.gpu.as_mut().expect("world() ran this frame");
        post.finish_with(encoder, screen, &gpu.world_group, None);
        let reset = gpu.reset || camera.reproject.is_none();
        let reproject = camera.reproject.unwrap_or(Mat4::IDENTITY);
        let object_motion = self
            .motion
            .as_mut()
            .expect("track_entities() ran this frame")
            .draw(
                device,
                queue,
                encoder,
                &camera.depth,
                camera.render_size,
                camera.jittered,
                camera.view_proj,
                reproject * camera.view_proj,
            );
        gpu.flow.dispatch(device, queue, encoder, &gpu.world, reset);
        gpu.interpolation.dispatch(
            device,
            queue,
            encoder,
            &interpolation::Inputs {
                color: &gpu.world,
                depth: &camera.depth,
                render_size: camera.render_size,
                jitter: camera.jitter,
                reproject,
                object_motion: &object_motion,
                near: camera.near,
                far: camera.far,
                fov_y: camera.fov_y,
                reset,
                flow_vectors: gpu.flow.vectors(),
                scene_change: gpu.flow.scene_change(),
            },
        );
        gpu.reset = false;
        if debug() == Some("motion") {
            post.finish_with(encoder, generated, &post.bind(device, &object_motion), None);
            return true;
        }
        if reset {
            post.finish_with(encoder, generated, &gpu.world_group, None);
        } else {
            post.finish_with(encoder, generated, &gpu.output_group, None);
        }
        !reset
    }

    /// Hand this frame's pair to the present thread, after the frame's submission: the
    /// made frame (when `made`) goes first.
    pub fn present(&mut self, real: wgpu::Texture, generated: wgpu::Texture, made: bool) {
        let now = Instant::now();
        if let Some(last) = self.last {
            // A real frame's time, smoothed: the made frame shows for half of it.
            let dt = (now - last).min(Duration::from_millis(250));
            self.interval = self.interval.mul_f32(0.9) + dt.mul_f32(0.1);
        }
        self.last = Some(now);
        self.real += 1;
        self.made += u32::from(made && !generated_only());
        if now - self.shown_since >= Duration::from_millis(500) {
            self.made_share = Some(self.made as f32 / self.real as f32);
            (self.real, self.made, self.shown_since) = (0, 0, now);
        }
        let Some(send) = self.presenter.as_ref().and_then(|p| p.send.as_ref()) else {
            return;
        };
        let pair = if generated_only() {
            Pair {
                first: None,
                second: generated,
                spare: Some(real),
                interval: self.interval,
            }
        } else if made {
            Pair {
                first: Some(generated),
                second: real,
                spare: None,
                interval: self.interval,
            }
        } else {
            Pair {
                first: None,
                second: real,
                spare: Some(generated),
                interval: self.interval,
            }
        };
        let _ = send.send(pair);
    }
}

/// A frame to draw into, like the swapchain's.
fn frame(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Frame generation frame"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &config.view_formats,
    })
}

/// One real frame's frames for the present thread: `first` (the made frame) now,
/// `second` (the real one) half an `interval` later; `spare` goes straight back.
struct Pair {
    first: Option<wgpu::Texture>,
    second: wgpu::Texture,
    spare: Option<wgpu::Texture>,
    interval: Duration,
}

/// The thread that copies frames to the swapchain and presents them on time.
struct Presenter {
    config: wgpu::SurfaceConfiguration,
    send: Option<mpsc::SyncSender<Pair>>,
    free: mpsc::Receiver<wgpu::Texture>,
    /// The surface gave something other than a frame: the renderer takes it back.
    failed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Presenter {
    fn start(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: Arc<wgpu::Surface<'static>>,
        window: Arc<winit::window::Window>,
        config: &wgpu::SurfaceConfiguration,
    ) -> Self {
        // One pair waiting at most: the renderer then waits as for a swapchain image.
        let (send, receive) = mpsc::sync_channel::<Pair>(1);
        let (give, free) = mpsc::channel();
        for _ in 0..FRAMES {
            let _ = give.send(frame(device, config));
        }
        let (device, queue) = (device.clone(), queue.clone());
        let vsync = config.present_mode == wgpu::PresentMode::Fifo;
        let failed = Arc::new(AtomicBool::new(false));
        let failed_here = failed.clone();
        let thread = thread::Builder::new()
            .name("Frame generation present".into())
            .spawn(move || {
                let show = |texture: &wgpu::Texture| {
                    if failed_here.load(Ordering::Relaxed) {
                        return;
                    }
                    // Anything but a frame (outdated, lost, occluded...): the renderer
                    // stops this thread and acquires itself, handling it as it does
                    // without frame generation. A suboptimal frame still shows.
                    let target = match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t) => t,
                        wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                            failed_here.store(true, Ordering::Relaxed);
                            t
                        }
                        _ => {
                            failed_here.store(true, Ordering::Relaxed);
                            return;
                        }
                    };
                    let mut encoder = device.create_command_encoder(&Default::default());
                    encoder.copy_texture_to_texture(
                        texture.as_image_copy(),
                        target.texture.as_image_copy(),
                        target.texture.size(),
                    );
                    queue.submit([encoder.finish()]);
                    if vsync {
                        window.pre_present_notify();
                    }
                    queue.present(target);
                };
                while let Ok(pair) = receive.recv() {
                    if let Some(first) = pair.first {
                        let shown = Instant::now();
                        show(&first);
                        let _ = give.send(first);
                        // ponytail: sleep pacing; a spin for the last half millisecond
                        // if it shows uneven on some systems.
                        let due = shown + pair.interval / 2;
                        if let Some(wait) = due.checked_duration_since(Instant::now()) {
                            thread::sleep(wait);
                        }
                    }
                    show(&pair.second);
                    let _ = give.send(pair.second);
                    if let Some(spare) = pair.spare {
                        let _ = give.send(spare);
                    }
                }
            })
            .expect("spawn the frame generation present thread");
        Self {
            config: config.clone(),
            send: Some(send),
            free,
            failed,
            thread: Some(thread),
        }
    }

    /// The surface failed it or the thread died (a panic).
    fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed) || self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        self.send = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::Vertex;
    use glam::Vec3;

    fn vertex(pos: Vec3) -> Vertex {
        Vertex {
            pos: pos.into(),
            ..bytemuck::Zeroable::zeroed()
        }
    }

    /// A swing lands halfway; a hand still on screen while the camera turns stays still;
    /// another item shows as it is.
    #[test]
    fn the_made_frame_s_hand_is_halfway_on_screen() {
        let mut fg = FrameGen::new();
        let mut out = Vec::new();
        fg.halfway_hand(
            &[vertex(Vec3::new(0.0, 0.0, -1.0))],
            Mat4::IDENTITY,
            &mut out,
        );
        fg.halfway_hand(
            &[vertex(Vec3::new(0.4, 0.0, -1.0))],
            Mat4::IDENTITY,
            &mut out,
        );
        assert!((Vec3::from(out[1].pos) - Vec3::new(0.2, 0.0, -1.0)).length() < 1e-6);
        // The camera turned: the vertex turned with it, the same on screen.
        let turned = Mat4::from_rotation_y(0.3);
        let on_screen = Vec3::new(0.4, 0.0, -1.0);
        let pos = turned.inverse().transform_point3(on_screen);
        fg.halfway_hand(&[vertex(pos)], turned, &mut out);
        assert!((Vec3::from(out[2].pos) - pos).length() < 1e-6);
        let mut other = vertex(Vec3::ZERO);
        other.uv = [0.5, 0.5];
        fg.halfway_hand(&[other], turned, &mut out);
        assert_eq!(out[3].pos, [0.0; 3]);
    }
}
