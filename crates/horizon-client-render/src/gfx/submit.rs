//! The frame's commands finished, submitted and presented on a thread of their own.
//!
//! wgpu records a pass's commands lazily: `CommandEncoder::finish` validates and
//! encodes the whole frame (about 0.1 ms with the submit), work the window's thread
//! would otherwise wait for before building the next frame. The renderer hands the
//! frame's encoder to [`Queue::submit_later`] and goes on; every use of the queue
//! ([`Deref`]) first waits for that frame to be submitted, so writes, copies and
//! submissions keep the order they had on one thread.
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// Whether frames go to the submit thread, unless `HORIZON_SUBMIT_THREAD` says (`0` off, `1` on):
/// on Linux, where it was measured faster (+32% FPS in a CPU-bound view on Vulkan, #5779). On
/// macOS's Metal it stalled: offscreen at render distance 12, a tenth to a half of the frame rate
/// with 20-180 ms p99 frames, against the same binary without it. Windows is not measured yet.
pub fn wanted() -> bool {
    match std::env::var("HORIZON_SUBMIT_THREAD").as_deref() {
        Ok("0") => false,
        Ok("1") => true,
        _ => cfg!(target_os = "linux"),
    }
}

/// Work to start right after a frame's submission (a readback's `map_async`).
pub type AfterSubmit = Box<dyn FnOnce() + Send>;

/// A frame for the submit thread: its commands and, in a window, the surface to present.
pub struct Job {
    pub encoder: wgpu::CommandEncoder,
    pub surface: Option<wgpu::SurfaceTexture>,
    pub after: Option<AfterSubmit>,
}

struct Thread {
    jobs: Option<mpsc::Sender<Job>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// What the submit thread sent back, once it is taken in by [`Queue::wait`].
struct Landing {
    done: mpsc::Receiver<wgpu::SubmissionIndex>,
    landed: Option<wgpu::SubmissionIndex>,
}

/// The device's queue, with frames submitted on the submit thread when it runs.
pub struct Queue {
    queue: wgpu::Queue,
    thread: Option<Thread>,
    /// A frame went to the submit thread and has not been waited for.
    pending: AtomicBool,
    landing: Mutex<Option<Landing>>,
    /// Nanoseconds the window's thread waited for the submit thread since the last
    /// [`take_waited`](Self::take_waited): the GPU or the display holding a frame back, not work.
    waited: AtomicU64,
}

impl Queue {
    /// `threaded`: frames may go to a submit thread (the renderer asks [`wanted`]).
    pub fn new(queue: wgpu::Queue, threaded: bool) -> Self {
        let mut this = Self {
            queue,
            thread: None,
            pending: AtomicBool::new(false),
            landing: Mutex::new(None),
            waited: AtomicU64::new(0),
        };
        if !threaded {
            return this;
        }
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        let queue = this.queue.clone();
        let spawned = std::thread::Builder::new()
            .name("Render submit".into())
            .spawn(move || {
                for job in job_rx {
                    let commands = job.encoder.finish();
                    let submission = queue.submit([commands]);
                    if let Some(after) = job.after {
                        after();
                    }
                    if let Some(surface) = job.surface {
                        queue.present(surface);
                    }
                    if done_tx.send(submission).is_err() {
                        break;
                    }
                }
            });
        match spawned {
            Ok(handle) => {
                this.thread = Some(Thread {
                    jobs: Some(jobs),
                    handle: Some(handle),
                });
                *this.lock() = Some(Landing { done, landed: None });
            }
            Err(e) => log::warn!("Horizon renderer: no submit thread ({e}); submitting in place"),
        }
        this
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Landing>> {
        self.landing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Whether frames can go to the submit thread.
    pub fn threaded(&self) -> bool {
        self.thread.is_some()
    }
    /// Finish, submit and present the frame on the submit thread, then run its `after`;
    /// given back, with nothing done, when there is none.
    pub fn submit_later(&self, job: Job) -> Result<(), Box<Job>> {
        let Some(jobs) = self.thread.as_ref().and_then(|t| t.jobs.as_ref()) else {
            return Err(Box::new(job));
        };
        self.wait();
        match jobs.send(job) {
            Ok(()) => {
                self.pending.store(true, Ordering::Release);
                Ok(())
            }
            Err(mpsc::SendError(job)) => Err(Box::new(job)),
        }
    }
    /// Wait until the frame on the submit thread is submitted and presented.
    pub fn wait(&self) {
        if !self.pending.load(Ordering::Acquire) {
            return;
        }
        let mut landing = self.lock();
        if !self.pending.load(Ordering::Acquire) {
            return;
        }
        if let Some(l) = landing.as_mut() {
            let started = Instant::now();
            // A thread that died (a wgpu panic on it) panics here, as it would have in place.
            let submission = l
                .done
                .recv()
                .expect("Horizon renderer: the submit thread stopped");
            l.landed = Some(submission);
            let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            self.waited.fetch_add(nanos, Ordering::Relaxed);
        }
        self.pending.store(false, Ordering::Release);
    }
    /// How long the window's thread waited for the submit thread since the last call: in a
    /// GPU-bound view, the previous frame's submission and present hold up every use of the
    /// queue (uploads in the frame's first half too), and that is waiting, not the frame's work.
    pub fn take_waited(&self) -> Duration {
        Duration::from_nanos(self.waited.swap(0, Ordering::Relaxed))
    }
    /// The submission of the frame [`submit_later`](Self::submit_later) took, once it is
    /// made; `None` if it was taken already.
    pub fn landed(&self) -> Option<wgpu::SubmissionIndex> {
        self.wait();
        self.lock().as_mut().and_then(|l| l.landed.take())
    }
}

impl Deref for Queue {
    type Target = wgpu::Queue;
    fn deref(&self) -> &wgpu::Queue {
        self.wait();
        &self.queue
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        self.wait();
        if let Some(thread) = &mut self.thread {
            thread.jobs.take();
            if let Some(handle) = thread.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn buffer(device: &wgpu::Device, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage,
            mapped_at_creation: false,
        })
    }

    #[test]
    fn writes_after_a_frame_on_the_thread_land_after_it() {
        let Some((device, queue)) = device() else {
            return;
        };
        let queue = Queue::new(queue, true);
        assert!(queue.threaded());
        use wgpu::BufferUsages as U;
        let source = buffer(&device, U::COPY_SRC | U::COPY_DST);
        let copy = buffer(&device, U::COPY_SRC | U::COPY_DST);
        let read = buffer(&device, U::MAP_READ | U::COPY_DST);
        queue.write_buffer(&source, 0, &[1, 2, 3, 4]);
        for frame in 0..3u8 {
            // The frame copies `source`; the next frame's write must not reach that copy.
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&source, 0, &copy, 0, 4);
            let job = Job {
                encoder,
                surface: None,
                after: None,
            };
            assert!(queue.submit_later(job).is_ok());
            queue.write_buffer(&source, 0, &[frame + 10; 4]);
            assert!(queue.landed().is_some(), "frame {frame} submitted");
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&copy, 0, &read, 0, 4);
            queue.submit([encoder.finish()]);
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let expected = if frame == 0 {
                [1, 2, 3, 4]
            } else {
                [frame + 9; 4]
            };
            assert_eq!(*read.slice(..).get_mapped_range().unwrap(), expected);
            read.unmap();
        }
    }
}
