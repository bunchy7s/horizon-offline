//! Players coming and going: joining P1's world through memory as a profile it picks
//! (`split_profiles`), dropping out.
use super::*;
use horizon_client::{gamepad::PadState, options_ui::Page};

/// Whether `game` shows its profile page now.
fn on_profiles(game: &Game<'_>) -> bool {
    game.screen == crate::game::Screen::Options && game.ui.options.page() == Page::SplitProfiles
}

/// Why a player's game left its server, when the server said (its Disconnected or error screen).
fn turned_away(game: &Game<'_>) -> Option<String> {
    use horizon_client::{multiplayer::Screen, title::Mode};
    match &game.title.as_ref()?.mode {
        Mode::Multiplayer(Screen::Disconnected { reason, .. }) => Some(reason.clone()),
        Mode::Error { message, .. } => Some(message.clone()),
        _ => None,
    }
}

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
    pub(super) fn free_seat(&self) -> Option<usize> {
        (2..=MAX_PLAYERS).find(|n| self.guests.iter().all(|g| g.seat != *n))
    }

    /// The splitscreen folder in the game folder: `profiles/<name>/` holds each profile.
    fn folder() -> Result<PathBuf> {
        Ok(std::env::current_dir()?.join("splitscreen"))
    }

    /// Add a player driven by `pad`: it picks a profile first, unless `name` is given (tests).
    pub(super) fn join(&mut self, pad: Option<PadId>, name: Option<String>) -> Result<()> {
        let seat = self.free_seat().ok_or("every seat is taken")?;
        // The first other player: P1 moves to the compositor now (back with nobody joined, if
        // this one can't: `about_to_wait`).
        if self.surfaces.is_empty() {
            self.enable()?;
        }
        // Until it has a profile, a scratch options file: never P1's.
        let options = Self::folder()?.join(".joining/options.txt");
        OPTIONS.set(Some(options.clone()));
        let game = self.start_guest(seat, &options, self.host.settings.clone());
        OPTIONS.set(None);
        let mut game = game?;
        if let Some(pad) = pad {
            // The press that joined it doesn't act in its game.
            self.fresh.push(pad);
        }
        // Over its title screen: Cancel leaves (`pick_profiles`).
        game.open_options_at(true, Page::SplitProfiles);
        let i = self.guests.len() + 1;
        self.guests.push(Guest {
            game,
            name: None,
            seat,
            options,
            pad,
            window: 1,
            widgets: Default::default(),
            entered: false,
            bumpers: [false; 2],
        });
        self.planned_window(i, seat);
        if let Some(name) = name {
            self.enter(i - 1, name)?;
        }
        Ok(())
    }

    /// The names playing now: P1's and the joined players'.
    fn taken(&self) -> Vec<String> {
        std::iter::once(self.host.options.player_name())
            .chain(self.guests.iter().filter_map(|g| g.name.clone()))
            .collect()
    }

    /// The profile pages just opened, filled: the list and who plays now.
    pub(super) fn sync_profiles(&mut self) {
        let taken = self.taken();
        for g in self.guests.iter_mut().filter(|g| on_profiles(&g.game)) {
            g.game.ui.options.split_profiles.taken = taken.clone();
        }
        let open = |g: &Guest<'_>| on_profiles(&g.game) && !g.game.ui.options.split_profiles.filled;
        if !self.guests.iter().any(open) {
            return;
        }
        let names = Self::folder()
            .and_then(|f| {
                Ok(horizon_client::split_profiles::store::names(
                    &f.join("profiles"),
                )?)
            })
            .map(|mut names| {
                names.sort_by_key(|n| n.to_lowercase());
                names
            })
            .unwrap_or_else(|e| {
                log::warn!("Horizon: splitscreen: the profiles: {e}");
                Vec::new()
            });
        for g in self.guests.iter_mut().filter(|g| open(g)) {
            let page = &mut g.game.ui.options.split_profiles;
            page.filled = true;
            page.names = names.clone();
            if let Some(me) = g.name.as_ref().filter(|me| !names.contains(me)) {
                page.names.insert(0, me.clone());
            }
            page.taken = taken.clone();
            page.current = g.name.clone();
        }
    }

    /// Guest `i`, playing, switches to profile `name` where it plays: out of the world as its
    /// old profile, in as the new one.
    fn switch(&mut self, i: usize, name: String) -> Result<()> {
        self.turn(i + 1);
        let left = self.guests[i].game.leave_server(None);
        self.turn(0);
        let old = self.guests[i].name.take();
        if let (Some(server), Some(old)) = (&self.host.integrated, &old) {
            server.leave_split(old);
        }
        left?;
        self.enter(i, name)
    }

    /// Guest `i` plays as profile `name`: its options, then into P1's world.
    pub(super) fn enter(&mut self, i: usize, name: String) -> Result<()> {
        if self.taken().iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            return Err(format!("{name} is already playing.").into());
        }
        // The page's names are player names; a test seat's (P2) isn't one, but no name leaves
        // the profiles folder.
        if !folder_name(&name) {
            return Err(format!("{name:?} can't be a profile.").into());
        }
        let profiles = Self::folder()?.join("profiles");
        if horizon_client::split_profiles::valid_name(&name) {
            horizon_client::split_profiles::store::restore(&profiles, &name)?;
        }
        let profile = profiles.join(&name);
        let options = profile.join("options.txt");
        let mut settings = if options.exists() {
            horizon_client::settings::Settings::load(&options)
        } else {
            std::fs::create_dir_all(&profile)?;
            self.host.settings.save(&options);
            self.host.settings.clone()
        };
        let pipe = self
            .host
            .integrated
            .as_ref()
            .ok_or("P1 is not in its world")?
            .join_split(&name)
            .ok_or("P1's world does not let players in")?;
        self.guests[i].options = options;
        self.turn(i + 1);
        let guest = &mut self.guests[i];
        // Its profile page closes as it goes in.
        guest.game.screen = crate::game::Screen::Playing;
        settings.extras.agent_control &= guest.game.options.control.is_some();
        guest.game.settings = settings;
        guest.game.options.player_name = Some(name.clone());
        guest.game.apply_settings();
        let joined = guest.game.connect_local(pipe, &name);
        self.turn(0);
        if let Err(e) = joined {
            // Let in for nothing: the world pauses again, the name is free.
            if let Some(server) = &self.host.integrated {
                server.leave_split(&name);
            }
            return Err(e);
        }
        self.guests[i].name = Some(name);
        self.guests[i].entered = false;
        Ok(())
    }

    /// Players' profile pages: a pick goes in, one that can't says why there; a forgotten
    /// profile is hidden; a joining player who cancels (or closes the page) leaves.
    pub(super) fn pick_profiles(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.guests.len() {
            if let Some(name) = self.guests[i]
                .game
                .ui
                .options
                .split_profiles
                .take_forgotten()
            {
                let hidden = if self.taken().iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    Err("This profile cannot be forgotten while it is playing.".into())
                } else {
                    Self::folder().and_then(|f| {
                        Ok(horizon_client::split_profiles::store::hide(
                            &f.join("profiles"),
                            &name,
                        )?)
                    })
                };
                match hidden {
                    Ok(()) => {
                        for guest in &mut self.guests {
                            guest.game.ui.options.split_profiles.filled = false;
                        }
                    }
                    Err(e) => self.refuse(i, e.to_string()),
                }
            }
            let joining = self.guests[i].name.is_none();
            match self.guests[i].game.ui.options.split_profiles.take_choice() {
                Some(Some(name)) => match (|| {
                    if self.taken().iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                        return Err(format!("{name} is already playing.").into());
                    }
                    if joining {
                        self.enter(i, name)
                    } else {
                        self.switch(i, name)
                    }
                })() {
                    Ok(()) => changed = true,
                    Err(why) => self.refuse(i, why.to_string()),
                },
                _ if joining && !on_profiles(&self.guests[i].game) => {
                    self.remove_guest(i);
                    changed = true;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        changed
    }

    /// Guest `i`'s pick can't play: its profile page says why (opened again if it had left the
    /// world for it).
    fn refuse(&mut self, i: usize, why: String) {
        log::warn!("Horizon: splitscreen: P{}: {why}", self.guests[i].seat);
        let game = &mut self.guests[i].game;
        if !on_profiles(game) {
            game.open_options_at(true, Page::SplitProfiles);
        }
        game.ui.options.split_profiles.refused = Some(why);
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
            host.settings.horizon.translucent_batching,
            // The pack its own game loads, from its profile.
            crate::game::shader_packs::picked_pack_draws_flat(&guest.settings),
        )?);
        guest.window = Some(window);
        guest.world_renderer_ready = true;
        guest.keyboard_mouse = false;
        guest.split_window = host.split_window;
        guest.apply_settings();
        // Its pad comes from the split (`route_pads`): it never opens the controllers itself.
        guest.pad.feed = Some(PadState::default());
        guest.ui.options.split_guest = true;
        Ok(guest)
    }

    /// Players who disconnected drop out; all of them once P1 has left its world. One the
    /// server turned away as it came in picks again, told why; one it sent away later (a kick)
    /// leaves, and P1 hears why. Whether any left.
    pub(super) fn drop_out(&mut self) -> bool {
        let p1_left = self.host.title.is_some() || self.host.integrated.is_none();
        let before = self.guests.len();
        if p1_left {
            self.host.split_hud.notice = None;
        }
        for k in (0..self.guests.len()).rev() {
            let g = &mut self.guests[k];
            g.entered |= g.game.remote.is_some() && g.game.player.is_some();
            // In the world: a pad P1 gave up is this player's for good.
            if g.entered
                && let Some(id) = g.pad
            {
                self.p1_pad.joined(id);
            }
            let g = &self.guests[k];
            // A player picking a profile has no connection yet.
            let playing = g.name.is_none() || g.game.remote.is_some();
            let why = (!p1_left && g.game.error.is_none() && !playing)
                .then(|| turned_away(&g.game))
                .flatten();
            if let (true, Some(name), Some(why)) = (g.entered, &g.name, &why) {
                self.host.split_hud.notice =
                    Some(Notice::new(format!("{name} could not stay"), why.clone()));
            } else if let Some(why) = why {
                let old = self.guests[k].name.take();
                if let (Some(server), Some(old)) = (&self.host.integrated, &old) {
                    server.leave_split(old);
                }
                self.refuse(k, why);
                continue;
            }
            if p1_left || !playing || g.game.error.is_some() {
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
        // Gone before it got into the world: a pad P1 gave up goes back to P1.
        if !guest.entered
            && let Some(id) = guest.pad
        {
            self.restore_given_up_pad(id);
        }
        if self.focus > k + 1 {
            self.focus -= 1;
        }
        if let (Some(server), Some(name)) = (&self.host.integrated, &guest.name) {
            server.leave_split(name);
        }
    }
}

/// A name that stays one folder inside `profiles`.
fn folder_name(name: &str) -> bool {
    !name.is_empty() && name != "." && !name.contains(['/', '\\']) && !name.contains("..")
}

#[cfg(test)]
mod tests {
    use super::folder_name;

    #[test]
    fn a_profile_name_never_leaves_the_profiles_folder() {
        assert!(folder_name("P2") && folder_name("Steve"));
        for bad in ["", ".", "..", "../x", "a/b", "a\\b"] {
            assert!(!folder_name(bad), "{bad:?}");
        }
    }
}
