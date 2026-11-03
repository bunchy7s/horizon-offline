//! A game started without its prepared files (the game build run by hand, not
//! through the launcher): a window that says where to get the launcher
//! (`horizon_client::no_resources`) and quits when it is closed. No loader, no
//! crash report: nothing went wrong in the game.
use super::*;
use horizon_client::no_resources::{Action, DOWNLOAD_URL, NoResources};
use winit::event::MouseButton;

struct Missing {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: UiResources,
    screen: NoResources,
    /// The pointer in physical pixels.
    pointer: (f64, f64),
    api: horizon_client::graphics::GraphicsApi,
    error: Option<Box<dyn std::error::Error>>,
}

/// Shows the screen until the player closes it.
pub(super) fn show(looked_in: &std::path::Path, settings: &Settings) -> Result<()> {
    let mut app = Missing {
        window: None,
        renderer: None,
        gui: UiResources {
            atlas: HudAtlas::build(looked_in)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::colour([0; 4])),
        },
        screen: NoResources::new(looked_in),
        pointer: (0.0, 0.0),
        api: settings.horizon.api,
        error: None,
    };
    horizon_client::hud::GUI_SCALE.store(settings.gui_scale, Ordering::Relaxed);
    EventLoop::new()?.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

impl Missing {
    fn size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    /// `f` over a Ui of the window's size, the cursor where the pointer is.
    fn with_ui<R>(
        &self,
        batch: &mut HudBatch,
        f: impl FnOnce(&mut horizon_client::ui_theme::Ui) -> R,
    ) -> R {
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        let mut ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, batch, cursor);
        let result = f(&mut ui);
        ui.finish();
        result
    }

    fn draw(&mut self) {
        let mut batch = HudBatch::new(self.size());
        self.with_ui(&mut batch, |ui| self.screen.draw(ui));
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render(&loading_frame(&batch)) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => log::warn!("Horizon: frame failed: {e}"),
        }
        // The frame was not shown (the first one often isn't): the glyphs it
        // rasterized never reached the atlas, so draw them again next frame.
        horizon_client::ui_theme::reset_atlas();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn click(&mut self, event_loop: &ActiveEventLoop) {
        let mut batch = HudBatch::new(self.size());
        match self.with_ui(&mut batch, |ui| self.screen.click(ui)) {
            Some(Action::Download) => packs::open_uri(DOWNLOAD_URL),
            Some(Action::Quit) => event_loop.exit(),
            None => {}
        }
    }
}

impl ApplicationHandler for Missing {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Horizon")
            .with_window_icon(horizon_client::window::icon())
            .with_inner_size(winit::dpi::LogicalSize::new(1040.0, 700.0));
        let made = event_loop
            .create_window(attributes)
            .map_err(Into::into)
            .and_then(|window| {
                let window = Arc::new(window);
                let renderer = loading_renderer(
                    window.clone(),
                    false,
                    Some(&self.gui.atlas),
                    Some(&self.gui.panorama),
                    None,
                    self.api,
                )?;
                Ok((window, renderer))
            });
        match made {
            Ok((window, renderer)) => {
                window.request_redraw();
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone().filter(|w| w.id() == id) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.click(event_loop),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                event_loop.exit()
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if horizon_threads::stop_requests() > 0 {
            event_loop.exit();
            return;
        }
        // Wake now and then for Ctrl+C in the console; redraws come with events.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(250),
        ));
    }
}
