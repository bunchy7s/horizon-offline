//! What a Wayland compositor says it shows a surface with (`wp_color_management_v1`): the
//! luminances of the surface's preferred image description. A compositor that manages
//! colour (KWin, Mutter, wlroots) maps HDR content onto the display relative to a
//! reference white and tone-maps above what the display reaches, so the game can't
//! measure the display through it: it asks instead.
//!
//! The watch runs on winit's own connection, on an event queue of its own, so winit's
//! events are left alone; winit's loop reads the socket, the watch only looks at what
//! landed on its queue.
use std::ffi::c_void;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{wl_registry, wl_surface::WlSurface},
};
use wayland_protocols::wp::color_management::v1::client::{
    wp_color_management_surface_feedback_v1::{self, WpColorManagementSurfaceFeedbackV1},
    wp_color_manager_v1::{self, WpColorManagerV1},
    wp_image_description_info_v1::{self, WpImageDescriptionInfoV1},
    wp_image_description_v1::{self, WpImageDescriptionV1},
};

/// A surface's preferred luminances, in cd/m² (nits).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Luminances {
    /// The reference white: where SDR white sits.
    pub reference: f32,
    /// The darkest and brightest the target display shows (`target_luminance`).
    pub target_min: f32,
    pub target_max: f32,
}

#[derive(Default)]
struct State {
    manager: Option<(u32, u32)>,
    /// The compositor changed the surface's preferred description since the last query.
    changed: bool,
    ready: Option<bool>,
    reference: Option<f32>,
    target: Option<(f32, f32)>,
    done: bool,
}

/// What a compositor says about one window, kept up to date: [`Watch::changed`] says
/// when it changes (another output, the player recalibrating the desktop).
///
/// Drop it before the window: its objects live on the window's connection.
pub struct Watch {
    queue: EventQueue<State>,
    state: State,
    manager: WpColorManagerV1,
    feedback: WpColorManagementSurfaceFeedbackV1,
}

impl Watch {
    /// Starts watching `window`'s surface, with what the compositor says now. None off
    /// Wayland, or where the compositor has no `wp_color_management_v1`.
    pub fn new(
        window: &(impl wgpu::rwh::HasDisplayHandle + wgpu::rwh::HasWindowHandle),
    ) -> Option<(Self, Option<Luminances>)> {
        use wgpu::rwh::{RawDisplayHandle, RawWindowHandle};
        let (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) = (
            window.display_handle().ok()?.as_raw(),
            window.window_handle().ok()?.as_raw(),
        ) else {
            return None;
        };
        // Safety: the window's live display and surface, borrowed for the call.
        let mut watch = unsafe { Self::on(d.display.as_ptr(), w.surface.as_ptr()) }?;
        let now = watch.query();
        Some((watch, now))
    }

    /// [`Watch::new`] on raw handles: `surface` (a `wl_surface*`) on `display` (a
    /// `wl_display*`).
    ///
    /// # Safety
    ///
    /// `display` and `surface` must be live libwayland objects of the same connection.
    unsafe fn on(display: *mut c_void, surface: *mut c_void) -> Option<Self> {
        // Safety: the caller's live display; a foreign one is never closed from here.
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let conn = Connection::from_backend(backend);
        // Safety: the caller's live wl_surface on this connection.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, id).ok()?;
        let mut queue: EventQueue<State> = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = State::default();
        let registry = conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state).ok()?;
        let (name, version) = state.manager?;
        let manager: WpColorManagerV1 = registry.bind(name, version.min(1), &qh, ());
        let feedback = manager.get_surface_feedback(&surface, &qh, ());
        Some(Self {
            queue,
            state,
            manager,
            feedback,
        })
    }

    /// What the compositor says now, if it changed since the last call. Cheap: it only
    /// looks at events the window's own loop already read.
    pub fn changed(&mut self) -> Option<Option<Luminances>> {
        self.queue.dispatch_pending(&mut self.state).ok()?;
        if !std::mem::take(&mut self.state.changed) {
            return None;
        }
        Some(self.query())
    }

    /// The surface's preferred description now.
    fn query(&mut self) -> Option<Luminances> {
        let qh = self.queue.handle();
        let state = &mut self.state;
        (state.ready, state.reference, state.target, state.done) = (None, None, None, false);
        let description = self.feedback.get_preferred(&qh, ());
        let found = (|| {
            while state.ready.is_none() {
                self.queue.blocking_dispatch(state).ok()?;
            }
            if state.ready != Some(true) {
                return None;
            }
            let _info = description.get_information(&qh, ());
            while !state.done {
                self.queue.blocking_dispatch(state).ok()?;
            }
            let (target_min, target_max) = state.target?;
            Some(Luminances {
                reference: state.reference?,
                target_min,
                target_max,
            })
        })();
        description.destroy();
        found
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.feedback.destroy();
        self.manager.destroy();
        let _ = self.queue.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == WpColorManagerV1::interface().name
        {
            state.manager = Some((name, version));
        }
    }
}
impl Dispatch<WpColorManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpColorManagerV1,
        _: wp_color_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WpColorManagementSurfaceFeedbackV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpColorManagementSurfaceFeedbackV1,
        event: wp_color_management_surface_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_color_management_surface_feedback_v1::Event::PreferredChanged { .. } = event {
            state.changed = true;
        }
    }
}
impl Dispatch<WpImageDescriptionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpImageDescriptionV1,
        event: wp_image_description_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_image_description_v1::Event::Ready { .. } => state.ready = Some(true),
            wp_image_description_v1::Event::Failed { .. } => state.ready = Some(false),
            _ => {}
        }
    }
}
impl Dispatch<WpImageDescriptionInfoV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpImageDescriptionInfoV1,
        event: wp_image_description_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_image_description_info_v1::Event::Luminances { reference_lum, .. } => {
                state.reference = Some(reference_lum as f32);
            }
            wp_image_description_info_v1::Event::TargetLuminance { min_lum, max_lum } => {
                state.target = Some((min_lum as f32 / 10_000.0, max_lum as f32));
            }
            wp_image_description_info_v1::Event::Done => state.done = true,
            _ => {}
        }
    }
}
