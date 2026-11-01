//! Players coming and going: joining P1's world through memory, dropping out.
use super::*;
use horizon_client::gamepad::PadState;

/// The `Game` `start` makes (~67 KB, by value) into a box, in a frame of its own that is gone
/// once it is: the copies stay out of the caller's frame, deep in the event loop.
#[inline(never)]
fn boxed(start: impl FnOnce() -> Result<Option<Game<'static>>>) -> Result<Box<Game<'static>>> {
    match start() {
        Ok(Some(game)) => Ok(Box::new(game)),
        Ok(None) => Err("the player's game did not start".into()),
        Err(e) => Err(e),
    }
}

impl Split<'_> {
    /// The first seat free (2 to `MAX_PLAYERS`).
    fn free_seat(&self) -> Option<usize> {
        (2..=MAX_PLAYERS).find(|n| self.guests.iter().all(|g| g.seat != *n))
    }

    /// A player joins P1's world as `P<seat>`, with its own options in the game folder's
    /// `splitscreen/profiles/P<seat>/` (P1's, the first time).
    pub(super) fn join(&mut self, pad: Option<PadId>) -> Result<()> {
        let seat = self.free_seat().ok_or("every seat is taken")?;
        let name = format!("P{seat}");
        let profile = std::env::current_dir()?.join(format!("splitscreen/profiles/{name}"));
        let options = profile.join("options.txt");
        let settings = if options.exists() {
            horizon_client::settings::Settings::load(&options)
        } else {
            std::fs::create_dir_all(&profile)?;
            self.host.settings.save(&options);
            self.host.settings.clone()
        };
        // The first other player: P1 moves to the compositor now (back with nobody joined, if
        // this one can't: `about_to_wait`).
        if self.surfaces.is_empty() {
            self.enable()?;
        }
        let pipe = self
            .host
            .integrated
            .as_ref()
            .ok_or("P1 is not in its world")?
            .join_split(&name)
            .ok_or("P1's world does not let players in")?;
        OPTIONS.set(Some(options.clone()));
        let game = self.start_guest(seat, &options, settings);
        OPTIONS.set(None);
        let joined = game.and_then(|mut game| {
            game.connect_local(pipe, &name)?;
            Ok(game)
        });
        let game = match joined {
            Ok(game) => game,
            Err(e) => {
                // Let in for nothing: the world pauses again, the name is free.
                if let Some(server) = &self.host.integrated {
                    server.leave_split(&name);
                }
                return Err(e);
            }
        };
        if let Some(pad) = pad {
            // The press that joined it doesn't act in its game.
            self.fresh.push(pad);
        }
        let i = self.guests.len() + 1;
        self.guests.push(Guest {
            game,
            name,
            seat,
            options,
            pad,
            window: 1,
            widgets: Default::default(),
        });
        self.planned_window(i, seat);
        Ok(())
    }

    /// A new player's game in its seat. Boxed as it comes out of `start_game`, and never
    /// inlined: a `Game` is ~67 KB, and copies of it in the event loop's frames overflowed
    /// Windows' 1 MB main stack.
    #[inline(never)]
    fn start_guest(
        &mut self,
        seat: usize,
        options_file: &Path,
        settings: horizon_client::settings::Settings,
    ) -> Result<Box<Game<'static>>> {
        // The players' games borrow P1's resources for the process's lifetime: one copy for all.
        if self.res.is_none() {
            self.res = Some(Box::leak(Box::new(self.host.res.clone())));
        }
        let res = self.res.ok_or("no resources")?;
        let host = &mut self.host;
        std::fs::create_dir_all(options_file.parent().ok_or("bad options path")?)?;
        let mut options = host.options.clone();
        options.world = None;
        options.server = None;
        // P1's frame-limited test runs end the process; a player's game doesn't.
        options.frames = None;
        options.world_frames = None;
        options.split_guest = true;
        options.offscreen = true;
        options.paced_by_split = true;
        options.player_name = Some(format!("P{seat}"));
        // P1's socket (`--control` or Agent Control) + the seat: every game in this process
        // would otherwise take the one default endpoint, P1's.
        options.control = host.control.as_ref().map(|c| {
            let mut p = c.path().as_os_str().to_owned();
            p.push(seat.to_string());
            p.into()
        });
        let mut settings = settings;
        settings.extras.agent_control &= options.control.is_some();
        let gui = UiResources::load(&options.resources, &options.resources)?;
        // P1's output, heard from this player's camera too (`Audio::share`).
        let audio = host.audio.share(seat as u8);
        let window = host.window.clone().ok_or("no window")?;
        let pack = host.pack.clone();
        let resources = host.options.resources.clone();
        let mut guest = boxed(|| {
            start_game(
                res,
                pack,
                gui,
                options,
                None,
                audio,
                resources,
                Instant::now(),
                None,
                Some(settings),
                None,
            )
        })?;
        guest.renderer = Some(build_renderer(
            window.clone(),
            true,
            &guest.pack,
            &guest.gui,
            host.renderer.as_ref(),
            None,
            host.settings.horizon.api,
        )?);
        guest.window = Some(window);
        guest.world_renderer_ready = true;
        guest.keyboard_mouse = false;
        guest.apply_settings();
        // Its pad comes from the split (`route_pads`): it never opens the controllers itself.
        guest.pad.feed = Some(PadState::default());
        Ok(guest)
    }

    /// Players who left their world (Disconnect) or failed drop out; all of them once P1 has
    /// left its world. Whether any did.
    pub(super) fn drop_out(&mut self) -> bool {
        let p1_left = self.host.title.is_some() || self.host.integrated.is_none();
        let before = self.guests.len();
        for k in (0..self.guests.len()).rev() {
            let game = &self.guests[k].game;
            if p1_left || game.remote.is_none() || game.error.is_some() {
                self.remove_guest(k);
            }
        }
        self.guests.len() != before
    }

    /// Guest `k` (player `k + 1`) leaves: keyboard and mouse stay with whoever had them (P1 if
    /// it was this one), and the server forgets the name (the world may pause again).
    pub(super) fn remove_guest(&mut self, k: usize) {
        // Its turn's buttons and options go back to their owners before the order shifts.
        self.turn(0);
        if self.focus == k + 1 {
            self.handoff(0);
            self.turn(0);
        }
        let guest = self.guests.remove(k);
        if self.focus > k + 1 {
            self.focus -= 1;
        }
        if let Some(server) = &self.host.integrated {
            server.leave_split(&guest.name);
        }
    }
}
